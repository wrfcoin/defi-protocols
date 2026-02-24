// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ILendingPool
/// @notice Interface for the WRFCoin lending pool protocol
interface ILendingPool {
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
        address indexed liquidator,
        address indexed borrower,
        address indexed asset,
        uint256 debtCovered,
        uint256 collateralSeized
    );

    /// @notice Deposit an asset into the lending pool as collateral
    /// @param asset The address of the ERC20 token to deposit
    /// @param amount The amount of tokens to deposit
    function deposit(address asset, uint256 amount) external;

    /// @notice Withdraw a previously deposited asset from the pool
    /// @param asset The address of the ERC20 token to withdraw
    /// @param amount The amount of tokens to withdraw
    function withdraw(address asset, uint256 amount) external;

    /// @notice Borrow an asset from the pool against deposited collateral
    /// @param asset The address of the ERC20 token to borrow
    /// @param amount The amount of tokens to borrow
    function borrow(address asset, uint256 amount) external;

    /// @notice Repay a borrowed asset to the pool
    /// @param asset The address of the ERC20 token to repay
    /// @param amount The amount of tokens to repay
    function repay(address asset, uint256 amount) external;

    /// @notice Liquidate an undercollateralized borrower's position
    /// @param borrower The address of the borrower to liquidate
    /// @param asset The address of the debt asset to repay
    function liquidate(address borrower, address asset) external;

    /// @notice Get the health factor of a user's position
    /// @param user The address of the user
    /// @return The health factor scaled by 1e18 (< 1e18 means liquidatable)
    function healthFactor(address user) external view returns (uint256);

    /// @notice Get the total reserves of an asset in the pool
    /// @param asset The address of the ERC20 token
    /// @return The total amount of the asset held in reserves
    function getReserves(address asset) external view returns (uint256);
}
