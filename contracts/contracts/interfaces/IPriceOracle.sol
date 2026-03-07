// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IPriceOracle
/// @notice Interface for an asset price oracle used by the LendingPool
/// @dev Implementations should return USD-denominated prices scaled by 1e18
interface IPriceOracle {
    /// @notice Get the USD price of an asset, scaled by 1e18
    /// @param asset The address of the ERC20 token
    /// @return The price of 1 whole token in USD, scaled by 1e18 (e.g., $100 = 100e18)
    function getPrice(address asset) external view returns (uint256);
}
