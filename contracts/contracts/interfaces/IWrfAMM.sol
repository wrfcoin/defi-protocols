// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IWrfAMM
/// @notice Canonical interface for WRFCoin automated market makers.
///
/// @dev Two AMM implementations exist in the WRFCoin ecosystem. They serve
///      different asset classes and must NOT share liquidity for the same pair:
///
///      ┌──────────────────────┬────────────────────────────────────────────────┐
///      │ Contract             │ Purpose                                        │
///      ├──────────────────────┼────────────────────────────────────────────────┤
///      │ AMMRouter            │ General-purpose multi-pair router.             │
///      │ (defi-protocols)     │ WRF/token pairs, governance tokens, yield-     │
///      │                      │ bearing assets. Configurable fee tiers          │
///      │                      │ (1/5/30/100 bps). LP shares tracked            │
///      │                      │ internally (no ERC-20 LP token).               │
///      ├──────────────────────┼────────────────────────────────────────────────┤
///      │ CarbonCreditAMM      │ Single-pair AMM for carbon credit assets.      │
///      │ (smart-contracts)    │ Flash loans, MEV protection, ERC-20 LP         │
///      │                      │ tokens, k-invariant validation. Uses           │
///      │                      │ Uniswap v2-style mint/burn/swap interface.      │
///      └──────────────────────┴────────────────────────────────────────────────┘
///
///      Fee model alignment: both contracts use basis-point fees
///      (e.g. feeBps = 30 means 0.3%). The historical 997/1000 ratio in
///      AMMRouter was replaced with the bps model in PR #48.
///
///      This interface covers the AMMRouter API. CarbonCreditAMM exposes a
///      lower-level Uniswap-v2 pair interface (transfer-then-mint/burn/swap)
///      and does not implement IWrfAMM directly. Integrate it via
///      `ICarbonCreditAMM` defined in
///      wrfcoin/smart-contracts — contracts/interfaces/ICarbonCreditAMM.sol.
interface IWrfAMM {
    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    /// @notice Emitted when a new pair is created with a specific fee tier
    event PairCreated(address indexed token0, address indexed token1, uint24 feeBps);

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

    // -----------------------------------------------------------------------
    // Pair creation
    // -----------------------------------------------------------------------

    /// @notice Create a pair with an explicit fee tier
    /// @dev Valid fee tiers: 1 bps (0.01%), 5 bps (0.05%), 30 bps (0.3%), 100 bps (1%)
    ///      If omitted, `addLiquidity` auto-creates with the DEFAULT_FEE_BPS on first deposit.
    /// @param tokenA Address of the first token
    /// @param tokenB Address of the second token
    /// @param feeBps Fee in basis points
    /// @return pairKey Canonical key identifying this pair
    function createPair(
        address tokenA,
        address tokenB,
        uint24 feeBps
    ) external returns (bytes32 pairKey);

    // -----------------------------------------------------------------------
    // Liquidity
    // -----------------------------------------------------------------------

    /// @notice Add liquidity to a token pair pool
    /// @param tokenA Address of the first token
    /// @param tokenB Address of the second token
    /// @param amountA Amount of tokenA to deposit
    /// @param amountB Amount of tokenB to deposit
    /// @return liquidity LP shares minted to the caller
    function addLiquidity(
        address tokenA,
        address tokenB,
        uint256 amountA,
        uint256 amountB
    ) external returns (uint256 liquidity);

    /// @notice Remove liquidity from a token pair pool
    /// @param tokenA Address of the first token
    /// @param tokenB Address of the second token
    /// @param liquidity LP shares to burn
    /// @return amountA Amount of tokenA returned
    /// @return amountB Amount of tokenB returned
    function removeLiquidity(
        address tokenA,
        address tokenB,
        uint256 liquidity
    ) external returns (uint256 amountA, uint256 amountB);

    // -----------------------------------------------------------------------
    // Swap
    // -----------------------------------------------------------------------

    /// @notice Swap one token for another through the AMM
    /// @param tokenIn Address of the token to sell
    /// @param tokenOut Address of the token to buy
    /// @param amountIn Amount of tokenIn to sell
    /// @param amountOutMin Minimum acceptable output (slippage guard)
    /// @return amountOut Actual amount of tokenOut received
    function swap(
        address tokenIn,
        address tokenOut,
        uint256 amountIn,
        uint256 amountOutMin
    ) external returns (uint256 amountOut);

    // -----------------------------------------------------------------------
    // View
    // -----------------------------------------------------------------------

    /// @notice Calculate expected output using the constant-product formula
    /// @param amountIn Input amount
    /// @param reserveIn Reserve of the input token
    /// @param reserveOut Reserve of the output token
    /// @param feeBps Fee in basis points for this pair
    /// @return amountOut Calculated output amount after fees
    function getAmountOut(
        uint256 amountIn,
        uint256 reserveIn,
        uint256 reserveOut,
        uint24 feeBps
    ) external pure returns (uint256 amountOut);

    /// @notice Get reserves for a token pair
    /// @param tokenA Address of the first token
    /// @param tokenB Address of the second token
    /// @return reserveA Reserve of tokenA
    /// @return reserveB Reserve of tokenB
    function getReserves(
        address tokenA,
        address tokenB
    ) external view returns (uint256 reserveA, uint256 reserveB);

    /// @notice Get the fee tier in basis points for a token pair
    /// @param tokenA Address of the first token
    /// @param tokenB Address of the second token
    /// @return feeBps Fee in basis points (0 if pair is uninitialized)
    function getPairFeeBps(
        address tokenA,
        address tokenB
    ) external view returns (uint24 feeBps);

    /// @notice Get LP share balance for a provider
    /// @param tokenA Address of the first token
    /// @param tokenB Address of the second token
    /// @param provider Address of the liquidity provider
    /// @return LP share balance
    function getLPBalance(
        address tokenA,
        address tokenB,
        address provider
    ) external view returns (uint256);
}
