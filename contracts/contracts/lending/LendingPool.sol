// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

/// @title LendingPool
/// @notice A lending pool contract for the WRFCoin DeFi protocol.
///         Users can deposit assets as collateral, borrow against them,
///         and positions can be liquidated when undercollateralized.
/// @dev Uses a simplified collateral factor and fixed price model for the scaffold.
///      Production deployments should integrate a proper price oracle.
contract LendingPool is ReentrancyGuard {
    using SafeERC20 for IERC20;

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    /// @notice Precision scalar used for ratio calculations (1e18)
    uint256 public constant PRECISION = 1e18;

    /// @notice Collateral factor — 75% of deposited value can be borrowed
    uint256 public constant COLLATERAL_FACTOR = 75e16; // 0.75 * 1e18

    /// @notice Liquidation bonus — liquidator receives 5% extra collateral
    uint256 public constant LIQUIDATION_BONUS = 105e16; // 1.05 * 1e18

    /// @notice Health factor threshold below which a position is liquidatable
    uint256 public constant LIQUIDATION_THRESHOLD = 1e18; // 1.0

    /// @notice Maximum number of unique assets a user can deposit or borrow
    uint256 public constant MAX_USER_ASSETS = 20;

    // -----------------------------------------------------------------------
    // State
    // -----------------------------------------------------------------------

    /// @notice Per-asset total reserves held in the pool
    mapping(address asset => uint256 totalDeposits) public reserves;

    /// @notice Per-asset total amount currently borrowed from the pool
    mapping(address asset => uint256 totalBorrowed) public totalBorrows;

    /// @notice Deposited balance per user per asset
    mapping(address user => mapping(address asset => uint256 amount)) public deposits;

    /// @notice Borrowed balance per user per asset
    mapping(address user => mapping(address asset => uint256 amount)) public borrows;

    /// @notice Set of assets a user has deposited (for health factor iteration)
    mapping(address user => address[]) internal _userDepositAssets;

    /// @notice Set of assets a user has borrowed (for health factor iteration)
    mapping(address user => address[]) internal _userBorrowAssets;

    /// @notice Quick lookup to avoid duplicate entries in the asset arrays
    mapping(address user => mapping(address asset => bool)) internal _hasDeposit;
    mapping(address user => mapping(address asset => bool)) internal _hasBorrow;

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    /// @notice Emitted when a user deposits an asset into the pool
    event Deposited(address indexed user, address indexed asset, uint256 amount);

    /// @notice Emitted when a user withdraws an asset from the pool
    event Withdrawn(address indexed user, address indexed asset, uint256 amount);

    /// @notice Emitted when a user borrows an asset from the pool
    event Borrowed(address indexed user, address indexed asset, uint256 amount);

    /// @notice Emitted when a user repays a borrowed asset
    event Repaid(address indexed user, address indexed asset, uint256 amount);

    /// @notice Emitted when a borrower's position is liquidated
    event Liquidated(
        bytes32 indexed positionId,
        uint256 collateralSeized,
        uint256 debtRepaid,
        address indexed liquidator,
        uint256 timestamp
    );

    // -----------------------------------------------------------------------
    // Errors
    // -----------------------------------------------------------------------

    error ZeroAmount();
    error InsufficientDeposit();
    error InsufficientReserves();
    error InsufficientBorrow();
    error HealthFactorTooLow();
    error PositionHealthy();
    error NoBorrowToLiquidate();
    error TooManyAssets();

    // -----------------------------------------------------------------------
    // External — Core operations
    // -----------------------------------------------------------------------

    /// @notice Deposit an asset into the lending pool as collateral
    /// @param asset The address of the ERC20 token to deposit
    /// @param amount The amount of tokens to deposit
    function deposit(address asset, uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();

        IERC20(asset).safeTransferFrom(msg.sender, address(this), amount);

        deposits[msg.sender][asset] += amount;
        reserves[asset] += amount;

        if (!_hasDeposit[msg.sender][asset]) {
            if (_userDepositAssets[msg.sender].length >= MAX_USER_ASSETS) revert TooManyAssets();
            _hasDeposit[msg.sender][asset] = true;
            _userDepositAssets[msg.sender].push(asset);
        }

        emit Deposited(msg.sender, asset, amount);
    }

    /// @notice Withdraw a previously deposited asset from the pool
    /// @param asset The address of the ERC20 token to withdraw
    /// @param amount The amount of tokens to withdraw
    function withdraw(address asset, uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();
        if (deposits[msg.sender][asset] < amount) revert InsufficientDeposit();
        if (reserves[asset] < amount) revert InsufficientReserves();

        deposits[msg.sender][asset] -= amount;
        reserves[asset] -= amount;

        // Ensure the withdrawal does not make the position undercollateralized
        if (_totalBorrowValue(msg.sender) > 0) {
            if (_computeHealthFactor(msg.sender) < LIQUIDATION_THRESHOLD) {
                revert HealthFactorTooLow();
            }
        }

        IERC20(asset).safeTransfer(msg.sender, amount);

        emit Withdrawn(msg.sender, asset, amount);
    }

    /// @notice Borrow an asset from the pool against deposited collateral
    /// @param asset The address of the ERC20 token to borrow
    /// @param amount The amount of tokens to borrow
    function borrow(address asset, uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();
        if (reserves[asset] < amount) revert InsufficientReserves();

        borrows[msg.sender][asset] += amount;
        totalBorrows[asset] += amount;
        reserves[asset] -= amount;

        if (!_hasBorrow[msg.sender][asset]) {
            if (_userBorrowAssets[msg.sender].length >= MAX_USER_ASSETS) revert TooManyAssets();
            _hasBorrow[msg.sender][asset] = true;
            _userBorrowAssets[msg.sender].push(asset);
        }

        // Verify health factor after borrowing
        if (_computeHealthFactor(msg.sender) < LIQUIDATION_THRESHOLD) {
            revert HealthFactorTooLow();
        }

        IERC20(asset).safeTransfer(msg.sender, amount);

        emit Borrowed(msg.sender, asset, amount);
    }

    /// @notice Repay a borrowed asset to the pool
    /// @param asset The address of the ERC20 token to repay
    /// @param amount The amount of tokens to repay
    function repay(address asset, uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();

        uint256 owed = borrows[msg.sender][asset];
        if (owed == 0) revert InsufficientBorrow();

        uint256 repayAmount = amount > owed ? owed : amount;

        IERC20(asset).safeTransferFrom(msg.sender, address(this), repayAmount);

        borrows[msg.sender][asset] -= repayAmount;
        totalBorrows[asset] -= repayAmount;
        reserves[asset] += repayAmount;

        emit Repaid(msg.sender, asset, repayAmount);
    }

    /// @notice Liquidate an undercollateralized borrower's position
    /// @dev The liquidator repays the borrower's debt and receives collateral
    ///      at a bonus rate. Only callable when borrower's health factor < 1.0.
    /// @param borrower The address of the borrower to liquidate
    /// @param asset The address of the debt asset to repay
    function liquidate(address borrower, address asset) external nonReentrant {
        uint256 debt = borrows[borrower][asset];
        if (debt == 0) revert NoBorrowToLiquidate();
        if (_computeHealthFactor(borrower) >= LIQUIDATION_THRESHOLD) revert PositionHealthy();

        // Liquidator repays the full debt
        IERC20(asset).safeTransferFrom(msg.sender, address(this), debt);

        borrows[borrower][asset] = 0;
        totalBorrows[asset] -= debt;
        reserves[asset] += debt;

        // Seize collateral with liquidation bonus (same asset for simplicity)
        uint256 collateralToSeize = (debt * LIQUIDATION_BONUS) / PRECISION;
        uint256 available = deposits[borrower][asset];
        if (collateralToSeize > available) {
            collateralToSeize = available;
        }

        deposits[borrower][asset] -= collateralToSeize;
        reserves[asset] -= collateralToSeize;

        IERC20(asset).safeTransfer(msg.sender, collateralToSeize);

        emit Liquidated(_positionId(borrower), collateralToSeize, debt, msg.sender, block.timestamp);
    }

    // -----------------------------------------------------------------------
    // External — View functions
    // -----------------------------------------------------------------------

    /// @notice Get the health factor of a user's position
    /// @param user The address of the user
    /// @return The health factor scaled by 1e18 (< 1e18 means liquidatable)
    function healthFactor(address user) external view returns (uint256) {
        return _computeHealthFactor(user);
    }

    /// @notice Get the total reserves of an asset in the pool
    /// @param asset The address of the ERC20 token
    /// @return The total amount of the asset held in reserves
    function getReserves(address asset) external view returns (uint256) {
        return reserves[asset];
    }

    /// @notice Get a user's deposit balance for a specific asset
    /// @param user The address of the user
    /// @param asset The address of the ERC20 token
    /// @return The deposited amount
    function getDeposit(address user, address asset) external view returns (uint256) {
        return deposits[user][asset];
    }

    /// @notice Get a user's borrow balance for a specific asset
    /// @param user The address of the user
    /// @param asset The address of the ERC20 token
    /// @return The borrowed amount
    function getBorrow(address user, address asset) external view returns (uint256) {
        return borrows[user][asset];
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// @dev Computes the health factor for a user.
    ///      health = (totalCollateral * collateralFactor) / totalBorrow
    ///      Uses a 1:1 price assumption for scaffold simplicity.
    ///      Returns type(uint256).max if the user has no borrows.
    function _computeHealthFactor(address user) internal view returns (uint256) {
        uint256 totalBorrow = _totalBorrowValue(user);
        if (totalBorrow == 0) return type(uint256).max;

        uint256 totalCollateral = _totalDepositValue(user);
        uint256 adjustedCollateral = (totalCollateral * COLLATERAL_FACTOR) / PRECISION;

        return (adjustedCollateral * PRECISION) / totalBorrow;
    }

    /// @dev Sum of all deposit balances for a user (1:1 price assumption)
    function _totalDepositValue(address user) internal view returns (uint256 total) {
        address[] storage assets = _userDepositAssets[user];
        for (uint256 i = 0; i < assets.length; i++) {
            total += deposits[user][assets[i]];
        }
    }

    /// @dev Sum of all borrow balances for a user (1:1 price assumption)
    function _totalBorrowValue(address user) internal view returns (uint256 total) {
        address[] storage assets = _userBorrowAssets[user];
        for (uint256 i = 0; i < assets.length; i++) {
            total += borrows[user][assets[i]];
        }
    }

    function _positionId(address borrower) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(borrower)));
    }
}
