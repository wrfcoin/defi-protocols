'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture } = require('@nomicfoundation/hardhat-network-helpers');

describe('YieldFarm Ownable2Step ownership transfer', function () {
  async function deployFarmFixture() {
    const [owner, newOwner, other] = await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const rewardToken = await MockERC20.deploy('Reward', 'RWD');
    const farm = await (
      await ethers.getContractFactory('YieldFarm')
    ).deploy(await rewardToken.getAddress(), ethers.parseEther('1'), owner.address);

    return { owner, newOwner, other, farm };
  }

  it('owner() returns deployer initially', async function () {
    const { owner, farm } = await loadFixture(deployFarmFixture);
    expect(await farm.owner()).to.equal(owner.address);
  });

  it('pendingOwner() is zero address initially', async function () {
    const { farm } = await loadFixture(deployFarmFixture);
    expect(await farm.pendingOwner()).to.equal(ethers.ZeroAddress);
  });

  it('transferOwnership() alone does not transfer — sets pending owner', async function () {
    const { owner, newOwner, farm } = await loadFixture(deployFarmFixture);

    await farm.connect(owner).transferOwnership(newOwner.address);

    // Owner has NOT changed yet
    expect(await farm.owner()).to.equal(owner.address);
    // Pending owner is set
    expect(await farm.pendingOwner()).to.equal(newOwner.address);
  });

  it('acceptOwnership() by pending owner completes the transfer', async function () {
    const { owner, newOwner, farm } = await loadFixture(deployFarmFixture);

    await farm.connect(owner).transferOwnership(newOwner.address);
    await farm.connect(newOwner).acceptOwnership();

    expect(await farm.owner()).to.equal(newOwner.address);
    expect(await farm.pendingOwner()).to.equal(ethers.ZeroAddress);
  });

  it('acceptOwnership() by non-pending-owner reverts', async function () {
    const { owner, newOwner, other, farm } = await loadFixture(deployFarmFixture);

    await farm.connect(owner).transferOwnership(newOwner.address);

    await expect(
      farm.connect(other).acceptOwnership()
    ).to.be.revertedWithCustomError(farm, 'OwnableUnauthorizedAccount');
  });

  it('owner() returns correct address at each stage of two-step transfer', async function () {
    const { owner, newOwner, farm } = await loadFixture(deployFarmFixture);

    // Stage 1: before transfer
    expect(await farm.owner()).to.equal(owner.address);
    expect(await farm.pendingOwner()).to.equal(ethers.ZeroAddress);

    // Stage 2: after transferOwnership (pending)
    await farm.connect(owner).transferOwnership(newOwner.address);
    expect(await farm.owner()).to.equal(owner.address);
    expect(await farm.pendingOwner()).to.equal(newOwner.address);

    // Stage 3: after acceptOwnership (complete)
    await farm.connect(newOwner).acceptOwnership();
    expect(await farm.owner()).to.equal(newOwner.address);
    expect(await farm.pendingOwner()).to.equal(ethers.ZeroAddress);
  });
});
