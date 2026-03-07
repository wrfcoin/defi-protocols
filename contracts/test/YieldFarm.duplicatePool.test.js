'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture } = require('@nomicfoundation/hardhat-network-helpers');

describe('security(duplicate-pool): YieldFarm duplicate LP token check', function () {
  async function deployFarmFixture() {
    const [owner] = await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const rewardToken = await MockERC20.deploy('Reward', 'RWD');
    const farm = await (
      await ethers.getContractFactory('YieldFarm')
    ).deploy(await rewardToken.getAddress(), ethers.parseEther('1'), owner.address);

    await rewardToken.mint(await farm.getAddress(), ethers.parseEther('100000'));

    return { owner, farm, rewardToken, MockERC20 };
  }

  it('allows adding a unique LP token', async function () {
    const { farm, MockERC20 } = await loadFixture(deployFarmFixture);
    const lp = await MockERC20.deploy('LP', 'LP');

    await expect(farm.addPool(100, await lp.getAddress())).to.not.be.reverted;
    expect(await farm.poolLength()).to.equal(1);
    expect(await farm.isPoolToken(await lp.getAddress())).to.equal(true);
  });

  it('reverts when adding the same LP token twice', async function () {
    const { farm, MockERC20 } = await loadFixture(deployFarmFixture);
    const lp = await MockERC20.deploy('LP', 'LP');
    const lpAddr = await lp.getAddress();

    await farm.addPool(100, lpAddr);
    await expect(farm.addPool(200, lpAddr)).to.be.revertedWith('Duplicate pool token');
  });

  it('pool count is correct after additions', async function () {
    const { farm, MockERC20 } = await loadFixture(deployFarmFixture);
    const lp1 = await MockERC20.deploy('LP1', 'LP1');
    const lp2 = await MockERC20.deploy('LP2', 'LP2');

    await farm.addPool(100, await lp1.getAddress());
    await farm.addPool(200, await lp2.getAddress());
    expect(await farm.poolLength()).to.equal(2);
  });

  it('multiple different tokens can all be added', async function () {
    const { farm, MockERC20 } = await loadFixture(deployFarmFixture);
    const tokens = [];

    for (let i = 0; i < 5; i++) {
      const lp = await MockERC20.deploy(`LP${i}`, `LP${i}`);
      tokens.push(lp);
      await farm.addPool(100, await lp.getAddress());
    }

    expect(await farm.poolLength()).to.equal(5);
    for (const token of tokens) {
      expect(await farm.isPoolToken(await token.getAddress())).to.equal(true);
    }
  });
});
