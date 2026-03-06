'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture } = require('@nomicfoundation/hardhat-network-helpers');

describe('security(oracle): LendingPool price oracle integration', function () {
  const PRECISION = ethers.parseEther('1'); // 1e18

  async function deployFixture() {
    const [owner, user, liquidator] = await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const LendingPool = await ethers.getContractFactory('LendingPool');
    const MockPriceOracle = await ethers.getContractFactory('MockPriceOracle');

    const pool = await LendingPool.deploy();
    const oracle = await MockPriceOracle.deploy();
    const poolAddr = await pool.getAddress();

    // Create two tokens with different "real" prices
    const tokenA = await MockERC20.deploy('TokenA', 'TKA'); // will be $100
    const tokenB = await MockERC20.deploy('TokenB', 'TKB'); // will be $1

    const supply = ethers.parseEther('1000000');

    // Fund user
    await tokenA.mint(user.address, supply);
    await tokenA.connect(user).approve(poolAddr, supply);
    await tokenB.mint(user.address, supply);
    await tokenB.connect(user).approve(poolAddr, supply);

    // Fund liquidator
    await tokenA.mint(liquidator.address, supply);
    await tokenA.connect(liquidator).approve(poolAddr, supply);
    await tokenB.mint(liquidator.address, supply);
    await tokenB.connect(liquidator).approve(poolAddr, supply);

    // Seed pool reserves for borrowing (owner deposits tokenB)
    await tokenB.mint(owner.address, supply);
    await tokenB.connect(owner).approve(poolAddr, supply);
    await pool.connect(owner).deposit(await tokenB.getAddress(), supply);

    // Also seed tokenA reserves
    await tokenA.mint(owner.address, supply);
    await tokenA.connect(owner).approve(poolAddr, supply);
    await pool.connect(owner).deposit(await tokenA.getAddress(), supply);

    return { owner, user, liquidator, pool, oracle, tokenA, tokenB };
  }

  // ---- Admin tests ----

  it('owner can set the price oracle', async function () {
    const { pool, oracle } = await loadFixture(deployFixture);
    await expect(pool.setPriceOracle(await oracle.getAddress())).to.not.be.reverted;
    expect(await pool.priceOracle()).to.equal(await oracle.getAddress());
  });

  it('non-owner cannot set the price oracle', async function () {
    const { pool, oracle, user } = await loadFixture(deployFixture);
    await expect(
      pool.connect(user).setPriceOracle(await oracle.getAddress())
    ).to.be.revertedWith('LendingPool: caller is not the owner');
  });

  it('emits OracleUpdated event', async function () {
    const { pool, oracle } = await loadFixture(deployFixture);
    const oracleAddr = await oracle.getAddress();
    await expect(pool.setPriceOracle(oracleAddr))
      .to.emit(pool, 'OracleUpdated')
      .withArgs(ethers.ZeroAddress, oracleAddr);
  });

  // ---- 1:1 fallback (no oracle) ----

  it('falls back to 1:1 pricing when no oracle is set', async function () {
    const { pool, user, tokenA, tokenB } = await loadFixture(deployFixture);

    // Deposit 100 tokenA, borrow 50 tokenB — without oracle, 1:1 assumed
    await pool.connect(user).deposit(await tokenA.getAddress(), ethers.parseEther('100'));
    await pool.connect(user).borrow(await tokenB.getAddress(), ethers.parseEther('50'));

    // health = (100 * 0.75) / 50 = 1.5e18
    const hf = await pool.healthFactor(user.address);
    expect(hf).to.equal(ethers.parseEther('1.5'));
  });

  // ---- Oracle-based health factor ----

  it('health factor uses oracle prices correctly', async function () {
    const { pool, oracle, user, tokenA, tokenB } = await loadFixture(deployFixture);

    const tokenAAddr = await tokenA.getAddress();
    const tokenBAddr = await tokenB.getAddress();

    // Set oracle: tokenA = $100, tokenB = $1
    await oracle.setPrice(tokenAAddr, ethers.parseEther('100'));
    await oracle.setPrice(tokenBAddr, ethers.parseEther('1'));
    await pool.setPriceOracle(await oracle.getAddress());

    // Deposit 1 tokenA (worth $100), borrow 1 tokenB (worth $1)
    await pool.connect(user).deposit(tokenAAddr, ethers.parseEther('1'));
    await pool.connect(user).borrow(tokenBAddr, ethers.parseEther('1'));

    // health = (1 * 100 * 0.75) / (1 * 1) = 75e18
    const hf = await pool.healthFactor(user.address);
    expect(hf).to.equal(ethers.parseEther('75'));
  });

  it('without oracle, 1:1 fallback treats all tokens equally', async function () {
    const { pool, user, tokenA, tokenB } = await loadFixture(deployFixture);

    // Deposit 100 tokenA, borrow 10 tokenB — without oracle, same as $1:$1
    await pool.connect(user).deposit(await tokenA.getAddress(), ethers.parseEther('100'));
    await pool.connect(user).borrow(await tokenB.getAddress(), ethers.parseEther('10'));

    // health = (100 * 0.75) / 10 = 7.5 — but with oracle (A=$100, B=$1),
    // it would be (10000 * 0.75) / 10 = 750. The 1:1 fallback is ~100x off.
    const hf = await pool.healthFactor(user.address);
    expect(hf).to.equal(ethers.parseEther('7.5'));
  });

  // ---- Liquidation triggered by oracle price move ----

  it('liquidation triggers correctly when oracle price moves', async function () {
    const { pool, oracle, user, liquidator, tokenA, tokenB } = await loadFixture(deployFixture);

    const tokenAAddr = await tokenA.getAddress();
    const tokenBAddr = await tokenB.getAddress();

    // Set initial prices: tokenA = $100, tokenB = $1
    await oracle.setPrice(tokenAAddr, ethers.parseEther('100'));
    await oracle.setPrice(tokenBAddr, ethers.parseEther('1'));
    await pool.setPriceOracle(await oracle.getAddress());

    // User deposits 1 tokenA ($100) and borrows 50 tokenB ($50)
    await pool.connect(user).deposit(tokenAAddr, ethers.parseEther('1'));
    await pool.connect(user).borrow(tokenBAddr, ethers.parseEther('50'));

    // health = (100 * 0.75) / 50 = 1.5 — healthy
    let hf = await pool.healthFactor(user.address);
    expect(hf).to.equal(ethers.parseEther('1.5'));

    // Crash tokenA price to $50
    await oracle.setPrice(tokenAAddr, ethers.parseEther('50'));

    // health = (50 * 0.75) / 50 = 0.75 — liquidatable
    hf = await pool.healthFactor(user.address);
    expect(hf).to.equal(ethers.parseEther('0.75'));
    expect(hf).to.be.lt(ethers.parseEther('1'));

    // Liquidator can now liquidate the position
    await expect(
      pool.connect(liquidator).liquidate(user.address, tokenBAddr)
    ).to.not.be.reverted;
  });

  it('liquidation reverts when position is healthy according to oracle', async function () {
    const { pool, oracle, user, liquidator, tokenA, tokenB } = await loadFixture(deployFixture);

    const tokenAAddr = await tokenA.getAddress();
    const tokenBAddr = await tokenB.getAddress();

    await oracle.setPrice(tokenAAddr, ethers.parseEther('100'));
    await oracle.setPrice(tokenBAddr, ethers.parseEther('1'));
    await pool.setPriceOracle(await oracle.getAddress());

    await pool.connect(user).deposit(tokenAAddr, ethers.parseEther('1'));
    await pool.connect(user).borrow(tokenBAddr, ethers.parseEther('50'));

    // health = 1.5 — healthy, cannot liquidate
    await expect(
      pool.connect(liquidator).liquidate(user.address, tokenBAddr)
    ).to.be.revertedWithCustomError(pool, 'PositionHealthy');
  });

  // ---- Multiple assets with different prices ----

  it('handles multiple collateral and borrow assets with different prices', async function () {
    const { pool, oracle, user, owner } = await loadFixture(deployFixture);

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const poolAddr = await pool.getAddress();

    const tokenC = await MockERC20.deploy('TokenC', 'TKC');
    const tokenD = await MockERC20.deploy('TokenD', 'TKD');
    const tokenCAddr = await tokenC.getAddress();
    const tokenDAddr = await tokenD.getAddress();
    const supply = ethers.parseEther('1000000');

    // Fund user
    await tokenC.mint(user.address, supply);
    await tokenC.connect(user).approve(poolAddr, supply);
    await tokenD.mint(user.address, supply);
    await tokenD.connect(user).approve(poolAddr, supply);

    // Seed reserves for tokenD
    await tokenD.mint(owner.address, supply);
    await tokenD.connect(owner).approve(poolAddr, supply);
    await pool.connect(owner).deposit(tokenDAddr, supply);

    // Set oracle: tokenC = $200, tokenD = $2
    await oracle.setPrice(tokenCAddr, ethers.parseEther('200'));
    await oracle.setPrice(tokenDAddr, ethers.parseEther('2'));
    await pool.setPriceOracle(await oracle.getAddress());

    // Deposit 10 tokenC ($2000 value), borrow 100 tokenD ($200 value)
    await pool.connect(user).deposit(tokenCAddr, ethers.parseEther('10'));
    await pool.connect(user).borrow(tokenDAddr, ethers.parseEther('100'));

    // health = (2000 * 0.75) / 200 = 7.5
    const hf = await pool.healthFactor(user.address);
    expect(hf).to.equal(ethers.parseEther('7.5'));
  });
});
