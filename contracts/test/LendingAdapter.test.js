'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');

describe('LendingAdapter', function () {
  it('encodes the canonical liquidation payload with Solidity ABI layout', async function () {
    const adapter = await (await ethers.getContractFactory('LendingAdapter')).deploy();
    const abiCoder = ethers.AbiCoder.defaultAbiCoder();

    const event = {
      positionId: '0x' + '00'.repeat(31) + '2a',
      collateralSeized: 210n,
      debtRepaid: 200n,
      liquidator: '0x00000000000000000000000000000000000000bb',
      timestamp: 3n,
    };

    const encoded = await adapter.encodeLiquidationEvent(event);
    const expected = abiCoder.encode(
      ['bytes32', 'uint256', 'uint256', 'address', 'uint256'],
      [
        event.positionId,
        event.collateralSeized,
        event.debtRepaid,
        event.liquidator,
        event.timestamp,
      ]
    );

    expect(encoded).to.equal(expected);
  });

  it('round-trips a canonical liquidation payload through the adapter', async function () {
    const adapter = await (await ethers.getContractFactory('LendingAdapter')).deploy();

    const event = {
      positionId: '0x' + '00'.repeat(31) + '2a',
      collateralSeized: 210n,
      debtRepaid: 200n,
      liquidator: '0x00000000000000000000000000000000000000bb',
      timestamp: 3n,
    };

    const encoded = await adapter.encodeLiquidationEvent(event);
    const decoded = await adapter.decodeLiquidationEvent(encoded);

    expect(decoded.positionId).to.equal(event.positionId);
    expect(decoded.collateralSeized).to.equal(event.collateralSeized);
    expect(decoded.debtRepaid).to.equal(event.debtRepaid);
    expect(decoded.liquidator).to.equal(event.liquidator);
    expect(decoded.timestamp).to.equal(event.timestamp);
  });
});
