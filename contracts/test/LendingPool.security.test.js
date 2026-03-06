'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture } = require('@nomicfoundation/hardhat-network-helpers');

describe('security(gas-limit): LendingPool MAX_USER_ASSETS limit', function () {
  const MAX_USER_ASSETS = 20;

  async function deployFixture() {
    const [owner, user, lp1, lp2] = await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const pool = await (await ethers.getContractFactory('LendingPool')).deploy();
    const poolAddr = await pool.getAddress();

    // Rotate liquidity providers to avoid any single LP hitting 20-asset limit
    const lpSigners = [owner, lp1, lp2];
    let lpIndex = 0;

    // Helper: create a funded & approved token
    // If seedReserves is true, an LP deposits into pool so borrow reserves exist
    async function createToken(name, seedReserves = false) {
      const token = await MockERC20.deploy(name, name);
      const supply = ethers.parseEther('10000');
      await token.mint(user.address, supply);
      await token.connect(user).approve(poolAddr, supply);

      if (seedReserves) {
        const lp = lpSigners[lpIndex % lpSigners.length];
        lpIndex++;
        await token.mint(lp.address, supply);
        await token.connect(lp).approve(poolAddr, supply);
        await pool.connect(lp).deposit(await token.getAddress(), supply);
      }

      return token;
    }

    return { owner, user, pool, createToken, MockERC20 };
  }

  // ---- deposit tests ----

  it('MAX_USER_ASSETS constant equals 20', async function () {
    const { pool } = await loadFixture(deployFixture);
    expect(await pool.MAX_USER_ASSETS()).to.equal(MAX_USER_ASSETS);
  });

  it('allows depositing a new asset when below MAX_USER_ASSETS', async function () {
    const { pool, user, createToken } = await loadFixture(deployFixture);
    const token = await createToken('T1');
    await expect(
      pool.connect(user).deposit(await token.getAddress(), ethers.parseEther('100'))
    ).to.not.be.reverted;
  });

  it('reverts with TooManyAssets when depositing the 21st unique asset', async function () {
    const { pool, user, createToken } = await loadFixture(deployFixture);

    // Deposit MAX_USER_ASSETS unique tokens
    for (let i = 0; i < MAX_USER_ASSETS; i++) {
      const token = await createToken(`D${i}`);
      await pool.connect(user).deposit(await token.getAddress(), ethers.parseEther('10'));
    }

    // The 21st unique deposit asset should revert
    const extraToken = await createToken('EXTRA');
    await expect(
      pool.connect(user).deposit(await extraToken.getAddress(), ethers.parseEther('10'))
    ).to.be.revertedWithCustomError(pool, 'TooManyAssets');
  });

  it('allows depositing MORE of an existing asset even at the limit', async function () {
    const { pool, user, createToken } = await loadFixture(deployFixture);

    const tokens = [];
    for (let i = 0; i < MAX_USER_ASSETS; i++) {
      const token = await createToken(`D${i}`);
      await pool.connect(user).deposit(await token.getAddress(), ethers.parseEther('10'));
      tokens.push(token);
    }

    // Depositing more of the first token (already tracked) should succeed
    await expect(
      pool.connect(user).deposit(await tokens[0].getAddress(), ethers.parseEther('5'))
    ).to.not.be.reverted;
  });

  // ---- borrow tests ----

  it('reverts with TooManyAssets when borrowing the 21st unique asset', async function () {
    const { pool, user, createToken } = await loadFixture(deployFixture);

    // First deposit a large collateral so health factor allows many borrows
    const collateral = await createToken('COL');
    await pool.connect(user).deposit(await collateral.getAddress(), ethers.parseEther('10000'));

    // Borrow MAX_USER_ASSETS unique tokens (small amounts)
    for (let i = 0; i < MAX_USER_ASSETS; i++) {
      const token = await createToken(`B${i}`, true);
      await pool.connect(user).borrow(await token.getAddress(), ethers.parseEther('1'));
    }

    // The 21st unique borrow asset should revert
    const extraToken = await createToken('BEXTRA', true);
    await expect(
      pool.connect(user).borrow(await extraToken.getAddress(), ethers.parseEther('1'))
    ).to.be.revertedWithCustomError(pool, 'TooManyAssets');
  });

  it('allows borrowing MORE of an existing asset even at the borrow limit', async function () {
    const { pool, user, createToken } = await loadFixture(deployFixture);

    const collateral = await createToken('COL');
    await pool.connect(user).deposit(await collateral.getAddress(), ethers.parseEther('10000'));

    const borrowTokens = [];
    for (let i = 0; i < MAX_USER_ASSETS; i++) {
      const token = await createToken(`B${i}`, true);
      await pool.connect(user).borrow(await token.getAddress(), ethers.parseEther('1'));
      borrowTokens.push(token);
    }

    // Borrowing more of the first borrow token should succeed
    await expect(
      pool.connect(user).borrow(await borrowTokens[0].getAddress(), ethers.parseEther('1'))
    ).to.not.be.reverted;
  });

  // ---- gas test ----

  it('health factor computation with 20 deposit + 20 borrow assets completes within gas', async function () {
    const { pool, user, createToken } = await loadFixture(deployFixture);

    // Deposit 20 unique assets
    for (let i = 0; i < MAX_USER_ASSETS; i++) {
      const token = await createToken(`A${i}`);
      await pool.connect(user).deposit(await token.getAddress(), ethers.parseEther('100'));
    }

    // Borrow 20 unique assets (small amounts to stay healthy)
    for (let i = 0; i < MAX_USER_ASSETS; i++) {
      const token = await createToken(`BB${i}`, true);
      await pool.connect(user).borrow(await token.getAddress(), ethers.parseEther('1'));
    }

    // healthFactor() should return without running out of gas
    const hf = await pool.healthFactor(user.address);
    expect(hf).to.be.gt(0);
  });
});
