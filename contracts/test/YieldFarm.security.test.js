'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture } = require('@nomicfoundation/hardhat-network-helpers');

describe('security(gas-limit): YieldFarm MAX_POOLS limit', function () {
  async function deployFarmFixture() {
    const [owner] = await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const rewardToken = await MockERC20.deploy('Reward', 'RWD');
    const farm = await (
      await ethers.getContractFactory('YieldFarm')
    ).deploy(await rewardToken.getAddress(), ethers.parseEther('1'), owner.address);

    // Fund farm with rewards
    await rewardToken.mint(await farm.getAddress(), ethers.parseEther('100000'));

    return { owner, farm, rewardToken, MockERC20 };
  }

  it('MAX_POOLS constant equals 100', async function () {
    const { farm } = await loadFixture(deployFarmFixture);
    expect(await farm.MAX_POOLS()).to.equal(100);
  });

  it('allows adding a pool when below MAX_POOLS', async function () {
    const { farm, MockERC20 } = await loadFixture(deployFarmFixture);
    const lp = await MockERC20.deploy('LP', 'LP');

    await expect(farm.addPool(100, await lp.getAddress())).to.not.be.reverted;
    expect(await farm.poolLength()).to.equal(1);
  });

  it('reverts with TooManyPools when adding pool at MAX_POOLS', async function () {
    const { farm, MockERC20 } = await loadFixture(deployFarmFixture);

    // Add MAX_POOLS pools
    for (let i = 0; i < 100; i++) {
      const lp = await MockERC20.deploy(`LP${i}`, `LP${i}`);
      await farm.addPool(10, await lp.getAddress());
    }

    expect(await farm.poolLength()).to.equal(100);

    // The 101st pool should revert
    const extraLp = await MockERC20.deploy('Extra', 'EXT');
    await expect(farm.addPool(10, await extraLp.getAddress()))
      .to.be.revertedWithCustomError(farm, 'TooManyPools');
  });

  it('setRewardPerSecond completes within gas limits with 50 pools', async function () {
    const { farm, MockERC20 } = await loadFixture(deployFarmFixture);

    for (let i = 0; i < 50; i++) {
      const lp = await MockERC20.deploy(`LP${i}`, `LP${i}`);
      await farm.addPool(10, await lp.getAddress());
    }

    // Should succeed without hitting gas limit
    const tx = await farm.setRewardPerSecond(ethers.parseEther('2'));
    const receipt = await tx.wait();

    expect(receipt.status).to.equal(1);
    expect(await farm.rewardPerSecond()).to.equal(ethers.parseEther('2'));
  });
});
