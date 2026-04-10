// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IWrfAMM} from "./IWrfAMM.sol";

/// @title IAMMRouter
/// @notice Interface for the WRFCoin AMMRouter contract.
/// @dev Extends IWrfAMM — the canonical WRFCoin AMM interface.
///      Prefer IWrfAMM for generic AMM integrations; use IAMMRouter only
///      when you need to reference AMMRouter-specific behaviour.
interface IAMMRouter is IWrfAMM {}
