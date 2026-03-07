'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture, time } = require('@nomicfoundation/hardhat-network-helpers');

describe('security(timelock): YieldFarm emission timelock', function () {
  const REWARD_PER_SECOND = ethers.parseEther('1');
  const NEW_RATE = ethers.parseEther('2');
  const TIMELOCK = 48 * 3600; // 48 hours
  const EXPIRY = 7 * 24 * 3600; // 7 days

  async function deployFarmFixture() {
    const [owner, alice, attacker] = await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const rewardToken = await MockERC20.deploy('Reward', 'RWD');
    const lpToken = await MockERC20.deploy('LP', 'LP');

    const farm = await (
      await ethers.getContractFactory('YieldFarm')
    ).deploy(await rewardToken.getAddress(), REWARD_PER_SECOND, owner.address);

    // Fund farm with rewards
    await rewardToken.mint(await farm.getAddress(), ethers.parseEther('100000'));

    // Add a pool
    await farm.addPool(100, await lpToken.getAddress());

    return { owner, alice, attacker, farm, rewardToken, lpToken };
  }

  describe('setRewardPerSecond removed', function () {
    it('does not have setRewardPerSecond function', async function () {
      const { farm } = await loadFixture(deployFarmFixture);
      expect(farm.setRewardPerSecond).to.be.undefined;
    });
  });

  describe('queueRewardPerSecond', function () {
    it('queues a new emission rate', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);

      expect(await farm.hasPendingRewardChange()).to.be.true;
      expect(await farm.pendingRewardPerSecond()).to.equal(NEW_RATE);
      // Original rate unchanged
      expect(await farm.rewardPerSecond()).to.equal(REWARD_PER_SECOND);
    });

    it('emits EmissionChangeQueued event', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      const tx = farm.queueRewardPerSecond(NEW_RATE);
      await expect(tx).to.emit(farm, 'EmissionChangeQueued');
    });

    it('reverts if a change is already queued', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);
      await expect(farm.queueRewardPerSecond(ethers.parseEther('3')))
        .to.be.revertedWithCustomError(farm, 'RewardChangeAlreadyQueued');
    });

    it('reverts if called by non-owner', async function () {
      const { farm, attacker } = await loadFixture(deployFarmFixture);

      await expect(farm.connect(attacker).queueRewardPerSecond(NEW_RATE))
        .to.be.revertedWithCustomError(farm, 'OwnableUnauthorizedAccount');
    });
  });

  describe('executeRewardPerSecond', function () {
    it('executes after 48-hour timelock', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);
      await time.increase(TIMELOCK);

      await farm.executeRewardPerSecond();

      expect(await farm.rewardPerSecond()).to.equal(NEW_RATE);
      expect(await farm.hasPendingRewardChange()).to.be.false;
    });

    it('emits EmissionChangeExecuted event with old and new rates', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);
      await time.increase(TIMELOCK);

      await expect(farm.executeRewardPerSecond())
        .to.emit(farm, 'EmissionChangeExecuted')
        .withArgs(REWARD_PER_SECOND, NEW_RATE);
    });

    it('reverts if timelock has not elapsed', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);
      // Advance less than 48 hours
      await time.increase(TIMELOCK - 10);

      await expect(farm.executeRewardPerSecond())
        .to.be.revertedWithCustomError(farm, 'TimelockNotElapsed');
    });

    it('reverts if timelock has expired (past 7-day window)', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);
      // Advance past timelock + expiry
      await time.increase(TIMELOCK + EXPIRY + 1);

      await expect(farm.executeRewardPerSecond())
        .to.be.revertedWithCustomError(farm, 'TimelockExpired');
    });

    it('reverts if no change is queued', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await expect(farm.executeRewardPerSecond())
        .to.be.revertedWithCustomError(farm, 'NoPendingRewardChange');
    });

    it('reverts if called by non-owner', async function () {
      const { farm, attacker } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);
      await time.increase(TIMELOCK);

      await expect(farm.connect(attacker).executeRewardPerSecond())
        .to.be.revertedWithCustomError(farm, 'OwnableUnauthorizedAccount');
    });
  });

  describe('cancelRewardPerSecond', function () {
    it('cancels a queued change', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);
      await farm.cancelRewardPerSecond();

      expect(await farm.hasPendingRewardChange()).to.be.false;
      expect(await farm.pendingRewardPerSecond()).to.equal(0);
      // Original rate unchanged
      expect(await farm.rewardPerSecond()).to.equal(REWARD_PER_SECOND);
    });

    it('emits EmissionChangeCancelled event', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);

      await expect(farm.cancelRewardPerSecond())
        .to.emit(farm, 'EmissionChangeCancelled')
        .withArgs(NEW_RATE);
    });

    it('allows queuing a new change after cancellation', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);
      await farm.cancelRewardPerSecond();

      const anotherRate = ethers.parseEther('5');
      await farm.queueRewardPerSecond(anotherRate);
      expect(await farm.pendingRewardPerSecond()).to.equal(anotherRate);
    });

    it('reverts if no change is queued', async function () {
      const { farm } = await loadFixture(deployFarmFixture);

      await expect(farm.cancelRewardPerSecond())
        .to.be.revertedWithCustomError(farm, 'NoPendingRewardChange');
    });

    it('reverts if called by non-owner', async function () {
      const { farm, attacker } = await loadFixture(deployFarmFixture);

      await farm.queueRewardPerSecond(NEW_RATE);

      await expect(farm.connect(attacker).cancelRewardPerSecond())
        .to.be.revertedWithCustomError(farm, 'OwnableUnauthorizedAccount');
    });
  });

  describe('constants', function () {
    it('EMISSION_TIMELOCK is 48 hours', async function () {
      const { farm } = await loadFixture(deployFarmFixture);
      expect(await farm.EMISSION_TIMELOCK()).to.equal(48 * 3600);
    });

    it('EMISSION_EXPIRY is 7 days', async function () {
      const { farm } = await loadFixture(deployFarmFixture);
      expect(await farm.EMISSION_EXPIRY()).to.equal(7 * 24 * 3600);
    });
  });
});
