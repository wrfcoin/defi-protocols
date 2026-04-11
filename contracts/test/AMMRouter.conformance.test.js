'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture } = require('@nomicfoundation/hardhat-network-helpers');

describe('AMMRouter canonical IWrfAMM/IAMMRouter conformance', function () {
  async function deployFixture() {
    const [owner, liquidityProvider, trader] = await ethers.getSigners();

    const router = await (await ethers.getContractFactory('AMMRouter')).deploy();
    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const tokenA = await MockERC20.deploy('Token A', 'TKNA');
    const tokenB = await MockERC20.deploy('Token B', 'TKNB');

    const mintAmount = 10_000_000n;
    await tokenA.mint(liquidityProvider.address, mintAmount);
    await tokenB.mint(liquidityProvider.address, mintAmount);
    await tokenA.mint(trader.address, mintAmount);
    await tokenB.mint(trader.address, mintAmount);

    await tokenA.connect(liquidityProvider).approve(await router.getAddress(), mintAmount);
    await tokenB.connect(liquidityProvider).approve(await router.getAddress(), mintAmount);
    await tokenA.connect(trader).approve(await router.getAddress(), mintAmount);
    await tokenB.connect(trader).approve(await router.getAddress(), mintAmount);

    return { owner, liquidityProvider, trader, router, tokenA, tokenB };
  }

  function selector(signature) {
    return ethers.id(signature).slice(0, 10);
  }

  it('exposes the canonical function selectors', async function () {
    const { router } = await loadFixture(deployFixture);
    const signatures = [
      'createPair(address,address,uint24)',
      'addLiquidity(address,address,uint256,uint256)',
      'removeLiquidity(address,address,uint256)',
      'swap(address,address,uint256,uint256)',
      'getAmountOut(uint256,uint256,uint256,uint24)',
      'getReserves(address,address)',
      'getPairFeeBps(address,address)',
      'getLPBalance(address,address,address)',
    ];

    for (const signature of signatures) {
      expect(router.interface.getFunction(signature).selector).to.equal(selector(signature));
    }
  });

  it('creates pairs with supported fee tiers and rejects unsupported ones', async function () {
    const { router, tokenA, tokenB } = await loadFixture(deployFixture);
    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const tokenC = await MockERC20.deploy('Token C', 'TKNC');
    const tokenD = await MockERC20.deploy('Token D', 'TKND');

    await expect(router.createPair(await tokenA.getAddress(), await tokenB.getAddress(), 1))
      .to.emit(router, 'PairCreated');
    await expect(router.createPair(await tokenC.getAddress(), await tokenD.getAddress(), 100))
      .to.emit(router, 'PairCreated');

    expect(
      await router.getPairFeeBps(await tokenA.getAddress(), await tokenB.getAddress())
    ).to.equal(1);
    expect(
      await router.getPairFeeBps(await tokenC.getAddress(), await tokenD.getAddress())
    ).to.equal(100);

    await expect(
      router.createPair(await tokenA.getAddress(), await tokenB.getAddress(), 7)
    ).to.be.revertedWithCustomError(router, 'InvalidFeeTier');
  });

  it('auto-creates a canonical pair on first liquidity add with the default fee tier', async function () {
    const { liquidityProvider, router, tokenA, tokenB } = await loadFixture(deployFixture);
    const tokenAAddress = await tokenA.getAddress();
    const tokenBAddress = await tokenB.getAddress();
    const [token0, token1] =
      tokenAAddress.toLowerCase() < tokenBAddress.toLowerCase()
        ? [tokenAAddress, tokenBAddress]
        : [tokenBAddress, tokenAAddress];

    await expect(
      router
        .connect(liquidityProvider)
        .addLiquidity(tokenAAddress, tokenBAddress, 1_000_000n, 4_000_000n)
    )
      .to.emit(router, 'PairCreated')
      .withArgs(token0, token1, 30)
      .and.to.emit(router, 'LiquidityAdded');

    expect(await router.getPairFeeBps(tokenAAddress, tokenBAddress)).to.equal(30);
    expect(
      await router.getLPBalance(
        tokenAAddress,
        tokenBAddress,
        liquidityProvider.address
      )
    ).to.equal(1_999_000n);
  });

  it('supports canonical liquidity add and remove flows with provider share accounting', async function () {
    const { liquidityProvider, router, tokenA, tokenB } = await loadFixture(deployFixture);

    const added = await router
      .connect(liquidityProvider)
      .addLiquidity(await tokenA.getAddress(), await tokenB.getAddress(), 1_000_000n, 4_000_000n);
    await added.wait();

    const lpBalance = await router.getLPBalance(
      await tokenA.getAddress(),
      await tokenB.getAddress(),
      liquidityProvider.address
    );
    expect(lpBalance).to.equal(1_999_000n);

    await expect(
      router
        .connect(liquidityProvider)
        .removeLiquidity(await tokenA.getAddress(), await tokenB.getAddress(), lpBalance)
    )
      .to.emit(router, 'LiquidityRemoved')
      .withArgs(
        liquidityProvider.address,
        await tokenA.getAddress(),
        await tokenB.getAddress(),
        999_500n,
        3_998_000n,
        lpBalance
      );

    const [reserveA, reserveB] = await router.getReserves(
      await tokenA.getAddress(),
      await tokenB.getAddress()
    );
    expect(reserveA).to.equal(500n);
    expect(reserveB).to.equal(2_000n);
    expect(
      await router.getLPBalance(
        await tokenA.getAddress(),
        await tokenB.getAddress(),
        liquidityProvider.address
      )
    ).to.equal(0);
  });

  it('uses the pair fee tier in canonical swap quotes and execution', async function () {
    const { liquidityProvider, trader, router, tokenA, tokenB } = await loadFixture(deployFixture);

    await router.createPair(await tokenA.getAddress(), await tokenB.getAddress(), 5);
    await router
      .connect(liquidityProvider)
      .addLiquidity(await tokenA.getAddress(), await tokenB.getAddress(), 2_000_000n, 2_000_000n);

    const quote = await router.getAmountOut(100_000n, 2_000_000n, 2_000_000n, 5);
    expect(quote).to.equal(95_192n);

    await expect(
      router
        .connect(trader)
        .swap(await tokenA.getAddress(), await tokenB.getAddress(), 100_000n, quote)
    )
      .to.emit(router, 'Swapped')
      .withArgs(trader.address, await tokenA.getAddress(), await tokenB.getAddress(), 100_000n, quote);

    const [reserveA, reserveB] = await router.getReserves(
      await tokenA.getAddress(),
      await tokenB.getAddress()
    );
    expect(reserveA).to.equal(2_100_000n);
    expect(reserveB).to.equal(1_904_808n);
    expect(await tokenB.balanceOf(trader.address)).to.equal(10_095_192n);
  });
});
