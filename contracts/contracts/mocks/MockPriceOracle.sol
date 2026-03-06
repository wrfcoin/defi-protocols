// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IPriceOracle} from "../interfaces/IPriceOracle.sol";

/// @title MockPriceOracle
/// @notice A mock price oracle for testing LendingPool oracle integration
contract MockPriceOracle is IPriceOracle {
    mapping(address => uint256) public prices;

    /// @notice Set the price for an asset
    /// @param asset The token address
    /// @param price The price scaled by 1e18
    function setPrice(address asset, uint256 price) external {
        prices[asset] = price;
    }

    /// @inheritdoc IPriceOracle
    function getPrice(address asset) external view override returns (uint256) {
        return prices[asset];
    }
}
