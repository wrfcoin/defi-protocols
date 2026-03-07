// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

/// @title YieldFarm
/// @notice A yield farming contract for the WRFCoin DeFi protocol.
///         Users stake LP tokens into pools and earn reward tokens distributed
///         at a configurable rate per second, weighted by allocation points.
/// @dev Reward distribution follows the standard "accumulated reward per share"
///      pattern. The owner can add pools and adjust allocation points.
contract YieldFarm is ReentrancyGuard, Ownable {
    using SafeERC20 for IERC20;

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    /// @notice Precision scalar for accumulated reward calculations
    uint256 public constant ACC_REWARD_PRECISION = 1e18;

    /// @notice Maximum number of pools to prevent unbounded iteration
    uint256 public constant MAX_POOLS = 100;

    /// @notice Timelock delay before a queued emission change can be executed
    uint256 public constant EMISSION_TIMELOCK = 48 hours;

    /// @notice Expiry window after which a queued emission change is no longer valid
    uint256 public constant EMISSION_EXPIRY = 7 days;

    // -----------------------------------------------------------------------
    // Types
    // -----------------------------------------------------------------------

    /// @notice Information about a farming pool
    struct PoolInfo {
        /// @dev The LP token accepted by this pool
        IERC20 lpToken;
        /// @dev Allocation points assigned to this pool (share of total rewards)
        uint256 allocPoint;
        /// @dev Last timestamp at which rewards were calculated
        uint256 lastRewardTime;
        /// @dev Accumulated rewards per share, scaled by ACC_REWARD_PRECISION
        uint256 accRewardPerShare;
        /// @dev Total LP tokens staked in this pool
        uint256 totalStaked;
    }

    /// @notice Information about a user's stake in a specific pool
    struct UserInfo {
        /// @dev Amount of LP tokens the user has staked
        uint256 amount;
        /// @dev Reward debt used to calculate pending rewards correctly
        uint256 rewardDebt;
    }

    // -----------------------------------------------------------------------
    // State
    // -----------------------------------------------------------------------

    /// @notice The ERC20 token distributed as rewards
    IERC20 public rewardToken;

    /// @notice Rewards distributed per second across all pools
    uint256 public rewardPerSecond;

    /// @notice Sum of all allocation points across all pools
    uint256 public totalAllocPoint;

    /// @notice Array of all farming pools
    PoolInfo[] public poolInfo;

    /// @notice Mapping of pool ID => user address => user info
    mapping(uint256 pid => mapping(address user => UserInfo)) public userInfo;

    /// @notice The pending reward-per-second value awaiting execution
    uint256 public pendingRewardPerSecond;

    /// @notice Timestamp when the pending emission change was queued
    uint256 public rewardChangeQueuedAt;

    /// @notice Whether there is a pending emission change in the queue
    bool public hasPendingRewardChange;

    /// @notice Tracks whether an LP token address has already been registered as a pool
    mapping(address => bool) public isPoolToken;

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    /// @notice Emitted when a user deposits LP tokens into a farm pool
    event Deposited(address indexed user, uint256 indexed pid, uint256 amount);

    /// @notice Emitted when a user withdraws LP tokens from a farm pool
    event Withdrawn(address indexed user, uint256 indexed pid, uint256 amount);

    /// @notice Emitted when a user harvests pending rewards
    event Harvested(address indexed user, uint256 indexed pid, uint256 amount);

    /// @notice Emitted when a new pool is added to the farm
    event PoolAdded(uint256 indexed pid, address indexed lpToken, uint256 allocPoint);

    /// @notice Emitted when a pool's allocation points are updated
    event PoolUpdated(uint256 indexed pid, uint256 allocPoint);

    /// @notice Emitted when an emission rate change is queued
    event EmissionChangeQueued(uint256 newRewardPerSecond, uint256 executeAfter);

    /// @notice Emitted when a queued emission rate change is executed
    event EmissionChangeExecuted(uint256 oldRewardPerSecond, uint256 newRewardPerSecond);

    /// @notice Emitted when a queued emission rate change is cancelled
    event EmissionChangeCancelled(uint256 cancelledRewardPerSecond);

    // -----------------------------------------------------------------------
    // Errors
    // -----------------------------------------------------------------------

    error ZeroAmount();
    error InsufficientStake();
    error InvalidPool();
    error TooManyPools();
    error NoPendingRewardChange();
    error TimelockNotElapsed();
    error TimelockExpired();
    error RewardChangeAlreadyQueued();

    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    /// @notice Initializes the yield farm with a reward token and emission rate
    /// @param _rewardToken The ERC20 token to distribute as rewards
    /// @param _rewardPerSecond The number of reward tokens emitted per second
    /// @param _owner The owner address that can manage pools
    constructor(
        IERC20 _rewardToken,
        uint256 _rewardPerSecond,
        address _owner
    ) Ownable(_owner) {
        rewardToken = _rewardToken;
        rewardPerSecond = _rewardPerSecond;
    }

    // -----------------------------------------------------------------------
    // External — Owner functions
    // -----------------------------------------------------------------------

    /// @notice Add a new LP token pool to the farm
    /// @param _allocPoint Allocation points for the new pool
    /// @param _lpToken The LP token that the pool accepts
    function addPool(uint256 _allocPoint, IERC20 _lpToken) external onlyOwner {
        if (poolInfo.length >= MAX_POOLS) revert TooManyPools();
        require(!isPoolToken[address(_lpToken)], "Duplicate pool token");

        isPoolToken[address(_lpToken)] = true;
        totalAllocPoint += _allocPoint;

        poolInfo.push(
            PoolInfo({
                lpToken: _lpToken,
                allocPoint: _allocPoint,
                lastRewardTime: block.timestamp,
                accRewardPerShare: 0,
                totalStaked: 0
            })
        );

        emit PoolAdded(poolInfo.length - 1, address(_lpToken), _allocPoint);
    }

    /// @notice Update the allocation points for a pool
    /// @param _pid The pool ID to update
    /// @param _allocPoint The new allocation points
    function setAllocPoint(uint256 _pid, uint256 _allocPoint) external onlyOwner {
        if (_pid >= poolInfo.length) revert InvalidPool();

        _updatePool(_pid);

        totalAllocPoint = totalAllocPoint - poolInfo[_pid].allocPoint + _allocPoint;
        poolInfo[_pid].allocPoint = _allocPoint;

        emit PoolUpdated(_pid, _allocPoint);
    }

    /// @notice Queue a new reward emission rate (subject to timelock)
    /// @param _rewardPerSecond The new reward tokens per second
    function queueRewardPerSecond(uint256 _rewardPerSecond) external onlyOwner {
        if (hasPendingRewardChange) revert RewardChangeAlreadyQueued();

        pendingRewardPerSecond = _rewardPerSecond;
        rewardChangeQueuedAt = block.timestamp;
        hasPendingRewardChange = true;

        emit EmissionChangeQueued(_rewardPerSecond, block.timestamp + EMISSION_TIMELOCK);
    }

    /// @notice Execute a previously queued emission rate change after the timelock has elapsed
    function executeRewardPerSecond() external onlyOwner {
        if (!hasPendingRewardChange) revert NoPendingRewardChange();
        if (block.timestamp < rewardChangeQueuedAt + EMISSION_TIMELOCK) revert TimelockNotElapsed();
        if (block.timestamp > rewardChangeQueuedAt + EMISSION_TIMELOCK + EMISSION_EXPIRY) revert TimelockExpired();

        // Update all pools before changing the rate
        uint256 length = poolInfo.length;
        for (uint256 i = 0; i < length; i++) {
            _updatePool(i);
        }

        uint256 oldRate = rewardPerSecond;
        rewardPerSecond = pendingRewardPerSecond;

        // Clear the pending change
        hasPendingRewardChange = false;
        pendingRewardPerSecond = 0;
        rewardChangeQueuedAt = 0;

        emit EmissionChangeExecuted(oldRate, rewardPerSecond);
    }

    /// @notice Cancel a previously queued emission rate change
    function cancelRewardPerSecond() external onlyOwner {
        if (!hasPendingRewardChange) revert NoPendingRewardChange();

        uint256 cancelled = pendingRewardPerSecond;

        hasPendingRewardChange = false;
        pendingRewardPerSecond = 0;
        rewardChangeQueuedAt = 0;

        emit EmissionChangeCancelled(cancelled);
    }

    // -----------------------------------------------------------------------
    // External — User functions
    // -----------------------------------------------------------------------

    /// @notice Deposit LP tokens into a farming pool to earn rewards
    /// @param pid The pool ID to deposit into
    /// @param amount The amount of LP tokens to deposit
    function deposit(uint256 pid, uint256 amount) external nonReentrant {
        if (pid >= poolInfo.length) revert InvalidPool();
        if (amount == 0) revert ZeroAmount();

        _updatePool(pid);

        PoolInfo storage pool = poolInfo[pid];
        UserInfo storage user = userInfo[pid][msg.sender];

        // Harvest any pending rewards before updating the stake
        if (user.amount > 0) {
            uint256 pending = _pendingReward(pool, user);
            if (pending > 0) {
                rewardToken.safeTransfer(msg.sender, pending);
                emit Harvested(msg.sender, pid, pending);
            }
        }

        pool.lpToken.safeTransferFrom(msg.sender, address(this), amount);

        user.amount += amount;
        pool.totalStaked += amount;
        user.rewardDebt = (user.amount * pool.accRewardPerShare) / ACC_REWARD_PRECISION;

        emit Deposited(msg.sender, pid, amount);
    }

    /// @notice Withdraw LP tokens from a farming pool
    /// @param pid The pool ID to withdraw from
    /// @param amount The amount of LP tokens to withdraw
    function withdraw(uint256 pid, uint256 amount) external nonReentrant {
        if (pid >= poolInfo.length) revert InvalidPool();
        if (amount == 0) revert ZeroAmount();

        PoolInfo storage pool = poolInfo[pid];
        UserInfo storage user = userInfo[pid][msg.sender];

        if (user.amount < amount) revert InsufficientStake();

        _updatePool(pid);

        // Harvest any pending rewards before updating the stake
        uint256 pending = _pendingReward(pool, user);
        if (pending > 0) {
            rewardToken.safeTransfer(msg.sender, pending);
            emit Harvested(msg.sender, pid, pending);
        }

        user.amount -= amount;
        pool.totalStaked -= amount;
        user.rewardDebt = (user.amount * pool.accRewardPerShare) / ACC_REWARD_PRECISION;

        pool.lpToken.safeTransfer(msg.sender, amount);

        emit Withdrawn(msg.sender, pid, amount);
    }

    /// @notice Harvest pending reward tokens from a farming pool
    /// @param pid The pool ID to harvest from
    function harvest(uint256 pid) external nonReentrant {
        if (pid >= poolInfo.length) revert InvalidPool();

        _updatePool(pid);

        PoolInfo storage pool = poolInfo[pid];
        UserInfo storage user = userInfo[pid][msg.sender];

        uint256 pending = _pendingReward(pool, user);
        user.rewardDebt = (user.amount * pool.accRewardPerShare) / ACC_REWARD_PRECISION;

        if (pending > 0) {
            rewardToken.safeTransfer(msg.sender, pending);
        }

        emit Harvested(msg.sender, pid, pending);
    }

    // -----------------------------------------------------------------------
    // External — View functions
    // -----------------------------------------------------------------------

    /// @notice View the pending reward for a user in a specific pool
    /// @param pid The pool ID to check
    /// @param _user The address of the user
    /// @return The amount of pending reward tokens
    function pendingReward(uint256 pid, address _user) external view returns (uint256) {
        if (pid >= poolInfo.length) revert InvalidPool();

        PoolInfo storage pool = poolInfo[pid];
        UserInfo storage user = userInfo[pid][_user];

        uint256 accRewardPerShare = pool.accRewardPerShare;

        if (block.timestamp > pool.lastRewardTime && pool.totalStaked > 0 && totalAllocPoint > 0) {
            uint256 elapsed = block.timestamp - pool.lastRewardTime;
            uint256 reward = (elapsed * rewardPerSecond * pool.allocPoint) / totalAllocPoint;
            accRewardPerShare += (reward * ACC_REWARD_PRECISION) / pool.totalStaked;
        }

        return (user.amount * accRewardPerShare) / ACC_REWARD_PRECISION - user.rewardDebt;
    }

    /// @notice Get the number of farming pools
    /// @return The total number of pools
    function poolLength() external view returns (uint256) {
        return poolInfo.length;
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// @dev Update reward variables for a pool to the current timestamp
    function _updatePool(uint256 pid) internal {
        PoolInfo storage pool = poolInfo[pid];

        if (block.timestamp <= pool.lastRewardTime) return;

        if (pool.totalStaked == 0 || totalAllocPoint == 0) {
            pool.lastRewardTime = block.timestamp;
            return;
        }

        uint256 elapsed = block.timestamp - pool.lastRewardTime;
        uint256 reward = (elapsed * rewardPerSecond * pool.allocPoint) / totalAllocPoint;

        pool.accRewardPerShare += (reward * ACC_REWARD_PRECISION) / pool.totalStaked;
        pool.lastRewardTime = block.timestamp;
    }

    /// @dev Calculate pending reward for a user in a pool (assumes pool is up-to-date)
    function _pendingReward(
        PoolInfo storage pool,
        UserInfo storage user
    ) internal view returns (uint256) {
        return (user.amount * pool.accRewardPerShare) / ACC_REWARD_PRECISION - user.rewardDebt;
    }
}
