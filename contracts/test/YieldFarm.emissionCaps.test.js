'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture, time } = require('@nomicfoundation/hardhat-network-helpers');

describe('economics(emission-caps): YieldFarm emission ceilings', function () {
  async function deployFarmFixture() {
    const [owner, alice] = await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const rewardToken = await MockERC20.deploy('Reward', 'RWD');
    const lpToken = await MockERC20.deploy('LP', 'LP');
    const farm = await (
      await ethers.getContractFactory('YieldFarm')
    ).deploy(await rewardToken.getAddress(), ethers.parseEther('1'), owner.address);

    await rewardToken.mint(await farm.getAddress(), ethers.parseEther('100000'));
    await lpToken.mint(alice.address, ethers.parseEther('1000'));
    await lpToken.connect(alice).approve(await farm.getAddress(), ethers.parseEther('1000'));
    await farm.addPool(100, await lpToken.getAddress());

    return { owner, alice, farm, rewardToken, lpToken };
  }

  it('rejects queued emission changes above the hard global cap', async function () {
    const { farm } = await loadFixture(deployFarmFixture);

    await expect(farm.queueRewardPerSecond(ethers.parseEther('101')))
      .to.be.revertedWithCustomError(farm, 'EmissionCapExceeded');
  });

  it('stores the hard global emission ceiling as a public constant', async function () {
    const { farm } = await loadFixture(deployFarmFixture);

    expect(await farm.MAX_GLOBAL_REWARD_PER_SECOND()).to.equal(ethers.parseEther('100'));
  });

  it('clamps pending rewards to the configured per-pool emission cap', async function () {
    const { farm, alice } = await loadFixture(deployFarmFixture);

    await farm.setPoolEmissionCap(0, ethers.parseEther('0.25'));
    await farm.connect(alice).deposit(0, ethers.parseEther('100'));
    await time.increase(10);

    expect(await farm.pendingReward(0, alice.address)).to.equal(ethers.parseEther('2.5'));
  });
});
