// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IYieldFarm
/// @notice Interface for the WRFCoin yield farming protocol
interface IYieldFarm {
    /// @notice Emitted when a user deposits LP tokens into a farm pool
    event Deposited(address indexed user, uint256 indexed pid, uint256 amount);

    /// @notice Emitted when a user withdraws LP tokens from a farm pool
    event Withdrawn(address indexed user, uint256 indexed pid, uint256 amount);

    /// @notice Emitted when a user harvests pending rewards
    event Harvested(address indexed user, uint256 indexed pid, uint256 amount);

    /// @notice Emitted when a new pool is added to the farm
    event PoolAdded(uint256 indexed pid, address indexed lpToken, uint256 allocPoint);

    /// @notice Deposit LP tokens into a farming pool to earn rewards
    /// @param pid The pool ID to deposit into
    /// @param amount The amount of LP tokens to deposit
    function deposit(uint256 pid, uint256 amount) external;

    /// @notice Withdraw LP tokens from a farming pool
    /// @param pid The pool ID to withdraw from
    /// @param amount The amount of LP tokens to withdraw
    function withdraw(uint256 pid, uint256 amount) external;

    /// @notice Harvest pending reward tokens from a farming pool
    /// @param pid The pool ID to harvest from
    function harvest(uint256 pid) external;

    /// @notice View the pending reward for a user in a specific pool
    /// @param pid The pool ID to check
    /// @param user The address of the user
    /// @return The amount of pending reward tokens
    function pendingReward(uint256 pid, address user) external view returns (uint256);
}
