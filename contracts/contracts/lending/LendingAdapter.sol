// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title LendingAdapter
/// @notice Encodes and decodes the canonical liquidation event payload shared with Rust.
contract LendingAdapter {
    struct CanonicalLiquidationEvent {
        bytes32 positionId;
        uint256 collateralSeized;
        uint256 debtRepaid;
        address liquidator;
        uint256 timestamp;
    }

    function encodeLiquidationEvent(
        CanonicalLiquidationEvent calldata liquidationEvent
    ) external pure returns (bytes memory) {
        return abi.encode(
            liquidationEvent.positionId,
            liquidationEvent.collateralSeized,
            liquidationEvent.debtRepaid,
            liquidationEvent.liquidator,
            liquidationEvent.timestamp
        );
    }

    function decodeLiquidationEvent(
        bytes calldata encoded
    ) external pure returns (CanonicalLiquidationEvent memory) {
        (
            bytes32 positionId,
            uint256 collateralSeized,
            uint256 debtRepaid,
            address liquidator,
            uint256 timestamp
        ) = abi.decode(encoded, (bytes32, uint256, uint256, address, uint256));

        return CanonicalLiquidationEvent({
            positionId: positionId,
            collateralSeized: collateralSeized,
            debtRepaid: debtRepaid,
            liquidator: liquidator,
            timestamp: timestamp
        });
    }
}
