// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IWrfAMM} from "../interfaces/IWrfAMM.sol";

/// @title AMMRouter
/// @notice An automated market maker router for the WRFCoin DeFi protocol.
///         Implements a constant product (x*y=k) AMM with a 0.3% trading fee.
///         Supports adding/removing liquidity and token swaps.
/// @dev Pair ordering is canonical (sorted by address) to avoid duplicate pools.
///      LP shares are tracked internally rather than via a separate ERC20 token.
contract AMMRouter is ReentrancyGuard, IWrfAMM {
    using SafeERC20 for IERC20;

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    /// @notice Default fee tier applied when a pool is created without an explicit fee (0.3%)
    uint24 public constant DEFAULT_FEE_BPS = 30;

    /// @notice Basis-point denominator for fee calculations
    uint256 public constant FEE_DENOMINATOR_BPS = 10_000;

    /// @notice Minimum liquidity locked on first deposit to prevent manipulation
    uint256 public constant MINIMUM_LIQUIDITY = 1000;

    // -----------------------------------------------------------------------
    // Types
    // -----------------------------------------------------------------------

    /// @notice Reserves and metadata for a token pair
    struct Pair {
        /// @dev Reserve of the lower-addressed token (token0)
        uint256 reserve0;
        /// @dev Reserve of the higher-addressed token (token1)
        uint256 reserve1;
        /// @dev Total LP shares outstanding for this pair
        uint256 totalSupply;
        /// @dev Trading fee in basis points (e.g. 30 = 0.3%). Zero means uninitialized.
        uint24 feeBps;
    }

    // -----------------------------------------------------------------------
    // State
    // -----------------------------------------------------------------------

    /// @notice Pair data keyed by the canonical pair hash
    mapping(bytes32 pairKey => Pair) public pairs;

    /// @notice LP share balances: pairKey => provider => shares
    mapping(bytes32 pairKey => mapping(address provider => uint256 shares)) public lpBalances;

    // -----------------------------------------------------------------------
    // Errors
    // -----------------------------------------------------------------------

    error ZeroAmount();
    error IdenticalTokens();
    error InsufficientLiquidity();
    error InsufficientOutputAmount();
    error InsufficientReserves();
    error InvalidReserves();
    /// @notice Thrown when the requested fee tier is not one of: 1, 5, 30, 100 bps
    error InvalidFeeTier(uint24 feeBps);
    /// @notice Thrown when attempting to create a pair that already exists
    error PairAlreadyExists();

    // -----------------------------------------------------------------------
    // External — Pair creation
    // -----------------------------------------------------------------------

    /// @notice Explicitly create a pair with a chosen fee tier
    /// @dev Valid fee tiers: 1 bps (0.01%), 5 bps (0.05%), 30 bps (0.3%), 100 bps (1%)
    ///      If a pair is never explicitly created, `addLiquidity` will auto-create it
    ///      with `DEFAULT_FEE_BPS` (30) on the first deposit.
    /// @param tokenA Address of the first token
    /// @param tokenB Address of the second token
    /// @param feeBps Fee in basis points
    /// @return pairKey The canonical key for this pair
    function createPair(
        address tokenA,
        address tokenB,
        uint24 feeBps
    ) external returns (bytes32 pairKey) {
        if (tokenA == tokenB) revert IdenticalTokens();
        if (!_isValidFeeTier(feeBps)) revert InvalidFeeTier(feeBps);

        (address token0, address token1, , ) = _sortTokens(tokenA, tokenB, 0, 0);
        pairKey = _pairKey(token0, token1);

        if (pairs[pairKey].feeBps != 0) revert PairAlreadyExists();

        pairs[pairKey].feeBps = feeBps;
        emit PairCreated(token0, token1, feeBps);
    }

    // -----------------------------------------------------------------------
    // External — Liquidity
    // -----------------------------------------------------------------------

    /// @notice Add liquidity to a token pair pool
    /// @dev On first deposit, LP shares equal sqrt(amountA * amountB) minus
    ///      MINIMUM_LIQUIDITY which is permanently locked. Subsequent deposits
    ///      mint shares proportional to the existing reserves.
    /// @param tokenA The address of the first token
    /// @param tokenB The address of the second token
    /// @param amountA The desired amount of tokenA to add
    /// @param amountB The desired amount of tokenB to add
    /// @return liquidity The amount of LP shares minted to the provider
    function addLiquidity(
        address tokenA,
        address tokenB,
        uint256 amountA,
        uint256 amountB
    ) external nonReentrant returns (uint256 liquidity) {
        if (amountA == 0 || amountB == 0) revert ZeroAmount();
        if (tokenA == tokenB) revert IdenticalTokens();

        (address token0, address token1, uint256 amount0, uint256 amount1) = _sortTokens(
            tokenA,
            tokenB,
            amountA,
            amountB
        );

        bytes32 key = _pairKey(token0, token1);
        Pair storage pair = pairs[key];

        IERC20(token0).safeTransferFrom(msg.sender, address(this), amount0);
        IERC20(token1).safeTransferFrom(msg.sender, address(this), amount1);

        if (pair.totalSupply == 0) {
            // First liquidity deposit — auto-initialize fee if createPair was not called
            if (pair.feeBps == 0) {
                pair.feeBps = DEFAULT_FEE_BPS;
            }
            liquidity = _sqrt(amount0 * amount1) - MINIMUM_LIQUIDITY;
            // Lock minimum liquidity to the zero address
            pair.totalSupply = MINIMUM_LIQUIDITY;
        } else {
            // Proportional deposit
            uint256 liquidity0 = (amount0 * pair.totalSupply) / pair.reserve0;
            uint256 liquidity1 = (amount1 * pair.totalSupply) / pair.reserve1;
            liquidity = liquidity0 < liquidity1 ? liquidity0 : liquidity1;
        }

        if (liquidity == 0) revert InsufficientLiquidity();

        pair.reserve0 += amount0;
        pair.reserve1 += amount1;
        pair.totalSupply += liquidity;
        lpBalances[key][msg.sender] += liquidity;

        emit LiquidityAdded(msg.sender, tokenA, tokenB, amountA, amountB, liquidity);
    }

    /// @notice Remove liquidity from a token pair pool
    /// @param tokenA The address of the first token
    /// @param tokenB The address of the second token
    /// @param liquidity The amount of LP shares to burn
    /// @return amountA The amount of tokenA returned to the provider
    /// @return amountB The amount of tokenB returned to the provider
    function removeLiquidity(
        address tokenA,
        address tokenB,
        uint256 liquidity
    ) external nonReentrant returns (uint256 amountA, uint256 amountB) {
        if (liquidity == 0) revert ZeroAmount();
        if (tokenA == tokenB) revert IdenticalTokens();

        (address token0, address token1, , ) = _sortTokens(tokenA, tokenB, 0, 0);
        bytes32 key = _pairKey(token0, token1);
        Pair storage pair = pairs[key];

        if (lpBalances[key][msg.sender] < liquidity) revert InsufficientLiquidity();

        uint256 amount0 = (liquidity * pair.reserve0) / pair.totalSupply;
        uint256 amount1 = (liquidity * pair.reserve1) / pair.totalSupply;

        if (amount0 == 0 || amount1 == 0) revert InsufficientLiquidity();

        lpBalances[key][msg.sender] -= liquidity;
        pair.totalSupply -= liquidity;
        pair.reserve0 -= amount0;
        pair.reserve1 -= amount1;

        IERC20(token0).safeTransfer(msg.sender, amount0);
        IERC20(token1).safeTransfer(msg.sender, amount1);

        // Map back to caller's token ordering
        if (tokenA == token0) {
            amountA = amount0;
            amountB = amount1;
        } else {
            amountA = amount1;
            amountB = amount0;
        }

        emit LiquidityRemoved(msg.sender, tokenA, tokenB, amountA, amountB, liquidity);
    }

    // -----------------------------------------------------------------------
    // External — Swap
    // -----------------------------------------------------------------------

    /// @notice Swap one token for another through the AMM
    /// @dev Uses the constant product formula with a 0.3% fee:
    ///      amountOut = (amountIn * 997 * reserveOut) / (reserveIn * 1000 + amountIn * 997)
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
    ) external nonReentrant returns (uint256 amountOut) {
        if (amountIn == 0) revert ZeroAmount();
        if (tokenIn == tokenOut) revert IdenticalTokens();

        (address token0, address token1, , ) = _sortTokens(tokenIn, tokenOut, 0, 0);
        bytes32 key = _pairKey(token0, token1);
        Pair storage pair = pairs[key];

        bool isToken0In = (tokenIn == token0);
        uint256 reserveIn = isToken0In ? pair.reserve0 : pair.reserve1;
        uint256 reserveOut = isToken0In ? pair.reserve1 : pair.reserve0;

        if (reserveIn == 0 || reserveOut == 0) revert InsufficientReserves();

        amountOut = getAmountOut(amountIn, reserveIn, reserveOut, pair.feeBps);
        if (amountOut < amountOutMin) revert InsufficientOutputAmount();
        if (amountOut > reserveOut) revert InsufficientReserves();

        IERC20(tokenIn).safeTransferFrom(msg.sender, address(this), amountIn);
        IERC20(tokenOut).safeTransfer(msg.sender, amountOut);

        // Update reserves
        if (isToken0In) {
            pair.reserve0 += amountIn;
            pair.reserve1 -= amountOut;
        } else {
            pair.reserve1 += amountIn;
            pair.reserve0 -= amountOut;
        }

        emit Swapped(msg.sender, tokenIn, tokenOut, amountIn, amountOut);
    }

    // -----------------------------------------------------------------------
    // External — View
    // -----------------------------------------------------------------------

    /// @notice Calculate the output amount for a given input using constant product formula
    /// @dev Formula: amountOut = (amountIn * (10000 - feeBps) * reserveOut)
    ///                         / (reserveIn * 10000 + amountIn * (10000 - feeBps))
    /// @param amountIn The input amount
    /// @param reserveIn The reserve of the input token
    /// @param reserveOut The reserve of the output token
    /// @param feeBps The fee in basis points for this pair (e.g. 30 = 0.3%)
    /// @return amountOut The calculated output amount after fees
    function getAmountOut(
        uint256 amountIn,
        uint256 reserveIn,
        uint256 reserveOut,
        uint24 feeBps
    ) public pure returns (uint256 amountOut) {
        if (amountIn == 0) revert ZeroAmount();
        if (reserveIn == 0 || reserveOut == 0) revert InvalidReserves();

        uint256 feeMultiplier = FEE_DENOMINATOR_BPS - feeBps;
        uint256 amountInWithFee = amountIn * feeMultiplier;
        uint256 numerator = amountInWithFee * reserveOut;
        uint256 denominator = (reserveIn * FEE_DENOMINATOR_BPS) + amountInWithFee;

        amountOut = numerator / denominator;
    }

    /// @notice Get reserves for a token pair
    /// @param tokenA The address of the first token
    /// @param tokenB The address of the second token
    /// @return reserveA Reserve of tokenA
    /// @return reserveB Reserve of tokenB
    function getReserves(
        address tokenA,
        address tokenB
    ) external view returns (uint256 reserveA, uint256 reserveB) {
        (address token0, address token1, , ) = _sortTokens(tokenA, tokenB, 0, 0);
        bytes32 key = _pairKey(token0, token1);
        Pair storage pair = pairs[key];

        if (tokenA == token0) {
            reserveA = pair.reserve0;
            reserveB = pair.reserve1;
        } else {
            reserveA = pair.reserve1;
            reserveB = pair.reserve0;
        }
    }

    /// @notice Get the fee tier in basis points for a token pair
    /// @param tokenA The address of the first token
    /// @param tokenB The address of the second token
    /// @return feeBps The fee in basis points (0 if pair has never been created)
    function getPairFeeBps(address tokenA, address tokenB) external view returns (uint24 feeBps) {
        (address token0, address token1, , ) = _sortTokens(tokenA, tokenB, 0, 0);
        feeBps = pairs[_pairKey(token0, token1)].feeBps;
    }

    /// @notice Get LP balance of a provider for a token pair
    /// @param tokenA The address of the first token
    /// @param tokenB The address of the second token
    /// @param provider The address of the liquidity provider
    /// @return The LP share balance
    function getLPBalance(
        address tokenA,
        address tokenB,
        address provider
    ) external view returns (uint256) {
        (address token0, address token1, , ) = _sortTokens(tokenA, tokenB, 0, 0);
        bytes32 key = _pairKey(token0, token1);
        return lpBalances[key][provider];
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// @dev Returns true for the four supported fee tiers: 1, 5, 30, 100 bps
    function _isValidFeeTier(uint24 feeBps) internal pure returns (bool) {
        return feeBps == 1 || feeBps == 5 || feeBps == 30 || feeBps == 100;
    }

    /// @dev Sort tokens into canonical order (lower address first) and reorder amounts
    function _sortTokens(
        address tokenA,
        address tokenB,
        uint256 amountA,
        uint256 amountB
    ) internal pure returns (address token0, address token1, uint256 amount0, uint256 amount1) {
        if (tokenA < tokenB) {
            return (tokenA, tokenB, amountA, amountB);
        } else {
            return (tokenB, tokenA, amountB, amountA);
        }
    }

    /// @dev Compute a unique key for a token pair
    function _pairKey(address token0, address token1) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(token0, token1));
    }

    /// @dev Babylonian method for integer square root
    function _sqrt(uint256 y) internal pure returns (uint256 z) {
        if (y > 3) {
            z = y;
            uint256 x = y / 2 + 1;
            while (x < z) {
                z = x;
                x = (y / x + x) / 2;
            }
        } else if (y != 0) {
            z = 1;
        }
    }
}
