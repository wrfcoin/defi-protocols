// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IAMMRouter
/// @notice Interface for the WRFCoin automated market maker router
interface IAMMRouter {
    /// @notice Emitted when liquidity is added to a pool
    event LiquidityAdded(
        address indexed provider,
        address indexed tokenA,
        address indexed tokenB,
        uint256 amountA,
        uint256 amountB,
        uint256 liquidity
    );

    /// @notice Emitted when liquidity is removed from a pool
    event LiquidityRemoved(
        address indexed provider,
        address indexed tokenA,
        address indexed tokenB,
        uint256 amountA,
        uint256 amountB,
        uint256 liquidity
    );

    /// @notice Emitted when a token swap is executed
    event Swapped(
        address indexed user,
        address indexed tokenIn,
        address indexed tokenOut,
        uint256 amountIn,
        uint256 amountOut
    );

    /// @notice Add liquidity to a token pair pool
    /// @param tokenA The address of the first token
    /// @param tokenB The address of the second token
    /// @param amountA The amount of tokenA to add
    /// @param amountB The amount of tokenB to add
    /// @return liquidity The amount of LP tokens minted
    function addLiquidity(
        address tokenA,
        address tokenB,
        uint256 amountA,
        uint256 amountB
    ) external returns (uint256 liquidity);

    /// @notice Remove liquidity from a token pair pool
    /// @param tokenA The address of the first token
    /// @param tokenB The address of the second token
    /// @param liquidity The amount of LP tokens to burn
    /// @return amountA The amount of tokenA returned
    /// @return amountB The amount of tokenB returned
    function removeLiquidity(
        address tokenA,
        address tokenB,
        uint256 liquidity
    ) external returns (uint256 amountA, uint256 amountB);

    /// @notice Swap one token for another through the AMM
    /// @param tokenIn The address of the token to sell
    /// @param tokenOut The address of the token to buy
    /// @param amountIn The amount of tokenIn to sell
    /// @param amountOutMin The minimum amount of tokenOut to receive (slippage protection)
    /// @return amountOut The actual amount of tokenOut received
    function swap(
        address tokenIn,
        address tokenOut,
        uint256 amountIn,
        uint256 amountOutMin
    ) external returns (uint256 amountOut);

    /// @notice Calculate the output amount for a given input using constant product formula
    /// @param amountIn The input amount
    /// @param reserveIn The reserve of the input token
    /// @param reserveOut The reserve of the output token
    /// @return amountOut The calculated output amount
    function getAmountOut(
        uint256 amountIn,
        uint256 reserveIn,
        uint256 reserveOut
    ) external pure returns (uint256 amountOut);
}
