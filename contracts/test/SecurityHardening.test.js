'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture, time } = require('@nomicfoundation/hardhat-network-helpers');

describe('security(reentrancy): callback attack coverage', function () {
  async function deployToken(name, symbol, contractName = 'MockERC20') {
    const Factory = await ethers.getContractFactory(contractName);
    return Factory.deploy(name, symbol);
  }

  async function deployLendingFixture() {
    const [owner, attacker] = await ethers.getSigners();
    const lendingPool = await (await ethers.getContractFactory('LendingPool')).deploy();
    const maliciousAsset = await deployToken('Malicious Asset', 'MAL', 'MaliciousCallbackToken');
    const normalAsset = await deployToken('Normal Asset', 'NORM');

    const mintAmount = ethers.parseEther('1000');
    await maliciousAsset.mint(attacker.address, mintAmount);
    await normalAsset.mint(attacker.address, mintAmount);

    await maliciousAsset.connect(attacker).approve(await lendingPool.getAddress(), mintAmount);
    await normalAsset.connect(attacker).approve(await lendingPool.getAddress(), mintAmount);

    return { owner, attacker, lendingPool, maliciousAsset, normalAsset, mintAmount };
  }

  async function deployAmmFixture() {
    const [, attacker] = await ethers.getSigners();
    const router = await (await ethers.getContractFactory('AMMRouter')).deploy();
    const maliciousToken = await deployToken('Malicious Token', 'MAL', 'MaliciousCallbackToken');
    const normalToken = await deployToken('Normal Token', 'NORM');
    const mintAmount = ethers.parseEther('100000');

    await maliciousToken.mint(attacker.address, mintAmount);
    await normalToken.mint(attacker.address, mintAmount);

    await maliciousToken.connect(attacker).approve(await router.getAddress(), mintAmount);
    await normalToken.connect(attacker).approve(await router.getAddress(), mintAmount);

    return { attacker, router, maliciousToken, normalToken, mintAmount };
  }

  async function deployYieldFixture(useMaliciousReward = false) {
    const [owner, attacker] = await ethers.getSigners();
    const rewardToken = await deployToken(
      useMaliciousReward ? 'Malicious Reward' : 'Reward Token',
      useMaliciousReward ? 'MRWD' : 'RWD',
      useMaliciousReward ? 'MaliciousCallbackToken' : 'MockERC20'
    );
    const lpToken = await deployToken(
      useMaliciousReward ? 'LP Token' : 'Malicious LP',
      useMaliciousReward ? 'LP' : 'MLP',
      useMaliciousReward ? 'MockERC20' : 'MaliciousCallbackToken'
    );
    const farm = await (await ethers.getContractFactory('YieldFarm')).deploy(
      await rewardToken.getAddress(),
      ethers.parseEther('1'),
      owner.address
    );

    const mintAmount = ethers.parseEther('10000');
    await rewardToken.mint(owner.address, mintAmount);
    await rewardToken.connect(owner).transfer(await farm.getAddress(), ethers.parseEther('5000'));

    await lpToken.mint(attacker.address, mintAmount);
    await lpToken.connect(attacker).approve(await farm.getAddress(), mintAmount);

    await farm.connect(owner).addPool(100, await lpToken.getAddress());

    return { owner, attacker, rewardToken, lpToken, farm };
  }

  async function deployYieldDepositFixture() {
    return deployYieldFixture(false);
  }

  async function deployYieldHarvestFixture() {
    return deployYieldFixture(true);
  }

  it('blocks callback reentrancy during lending deposits', async function () {
    const { attacker, lendingPool, maliciousAsset } = await loadFixture(deployLendingFixture);

    const reentryCall = lendingPool.interface.encodeFunctionData('deposit', [
      await maliciousAsset.getAddress(),
      1n,
    ]);
    await maliciousAsset.configureReentrancy(
      await lendingPool.getAddress(),
      await lendingPool.getAddress(),
      reentryCall
    );

    await expect(
      lendingPool.connect(attacker).deposit(await maliciousAsset.getAddress(), ethers.parseEther('10'))
    ).to.be.reverted;
  });

  it('blocks callback reentrancy during AMM liquidity adds', async function () {
    const { attacker, router, maliciousToken, normalToken } = await loadFixture(deployAmmFixture);

    const reentryCall = router.interface.encodeFunctionData('addLiquidity', [
      await maliciousToken.getAddress(),
      await normalToken.getAddress(),
      1n,
      1n,
    ]);
    await maliciousToken.configureReentrancy(
      await router.getAddress(),
      await router.getAddress(),
      reentryCall
    );

    await expect(
      router
        .connect(attacker)
        .addLiquidity(
          await maliciousToken.getAddress(),
          await normalToken.getAddress(),
          ethers.parseEther('1000'),
          ethers.parseEther('1000')
        )
    ).to.be.reverted;
  });

  it('blocks callback reentrancy during AMM swaps', async function () {
    const { attacker, router, maliciousToken, normalToken } = await loadFixture(deployAmmFixture);

    await router
      .connect(attacker)
      .addLiquidity(
        await maliciousToken.getAddress(),
        await normalToken.getAddress(),
        ethers.parseEther('5000'),
        ethers.parseEther('5000')
      );

    const reentryCall = router.interface.encodeFunctionData('swap', [
      await maliciousToken.getAddress(),
      await normalToken.getAddress(),
      1n,
      0n,
    ]);
    await maliciousToken.configureReentrancy(
      await router.getAddress(),
      await router.getAddress(),
      reentryCall
    );

    await expect(
      router
        .connect(attacker)
        .swap(
          await maliciousToken.getAddress(),
          await normalToken.getAddress(),
          ethers.parseEther('10'),
          0
        )
    ).to.be.reverted;
  });

  it('blocks callback reentrancy during farm deposits', async function () {
    const { attacker, lpToken, farm } = await loadFixture(deployYieldDepositFixture);

    const reentryCall = farm.interface.encodeFunctionData('deposit', [0, 1n]);
    await lpToken.configureReentrancy(
      await farm.getAddress(),
      await farm.getAddress(),
      reentryCall
    );

    await expect(farm.connect(attacker).deposit(0, ethers.parseEther('50'))).to.be.reverted;
  });

  it('blocks callback reentrancy during reward harvests', async function () {
    const { attacker, rewardToken, lpToken, farm } = await loadFixture(
      deployYieldHarvestFixture
    );

    await farm.connect(attacker).deposit(0, ethers.parseEther('100'));
    await time.increase(10);

    const reentryCall = farm.interface.encodeFunctionData('harvest', [0]);
    await rewardToken.configureReentrancy(
      await farm.getAddress(),
      await farm.getAddress(),
      reentryCall
    );

    await expect(farm.connect(attacker).harvest(0)).to.be.reverted;

    await rewardToken.disableReentrancy();
    expect(await lpToken.balanceOf(attacker.address)).to.equal(
      ethers.parseEther('9900')
    );
  });
});
