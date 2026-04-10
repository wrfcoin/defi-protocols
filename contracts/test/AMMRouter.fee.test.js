// AMMRouter fee-model conformance test
//
// Verifies that AMMRouter's basis-point fee formula is consistent with the
// fee model documented in ICarbonCreditAMM (wrfcoin/smart-contracts).
// Both AMMs use: amountInWithFee = amountIn * (10_000 - feeBps)
//               amountOut = (amountInWithFee * reserveOut) /
//                           (reserveIn * 10_000 + amountInWithFee)
//
// This test lives in defi-protocols and exercises AMMRouter directly.
// The same formula is enforced in smart-contracts via ICarbonCreditAMM
// and CarbonCreditAMM.getAmountOut().

const { expect } = require("chai");
const { ethers } = require("hardhat");

describe("AMMRouter — fee-model conformance", function () {
  let router;
  let tokenA, tokenB;
  let deployer, lp, swapper;

  const FEE_DENOMINATOR = 10_000n;

  // Reference formula — mirrors CarbonCreditAMM.getAmountOut and AMMRouter.getAmountOut
  function expectedAmountOut(amountIn, reserveIn, reserveOut, feeBps) {
    const amountInWithFee = BigInt(amountIn) * (FEE_DENOMINATOR - BigInt(feeBps));
    const numerator = amountInWithFee * BigInt(reserveOut);
    const denominator = BigInt(reserveIn) * FEE_DENOMINATOR + amountInWithFee;
    return numerator / denominator;
  }

  beforeEach(async function () {
    [deployer, lp, swapper] = await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory("MockERC20");
    tokenA = await MockERC20.deploy("Token A", "TKA");
    tokenB = await MockERC20.deploy("Token B", "TKB");

    const AMMRouter = await ethers.getContractFactory("AMMRouter");
    router = await AMMRouter.deploy();

    const liquidity = ethers.parseEther("1000");
    await tokenA.mint(lp.address, liquidity);
    await tokenB.mint(lp.address, liquidity);
    await tokenA.connect(lp).approve(router.target, liquidity);
    await tokenB.connect(lp).approve(router.target, liquidity);
    // Auto-creates pair at DEFAULT_FEE_BPS = 30
    await router.connect(lp).addLiquidity(tokenA.target, tokenB.target, liquidity, liquidity);
  });

  describe("getAmountOut — formula consistency across fee tiers", function () {
    const RESERVE = ethers.parseEther("1000");
    const AMOUNT_IN = ethers.parseEther("10");

    for (const feeBps of [1, 5, 30, 100]) {
      it(`feeBps=${feeBps}: matches reference constant-product formula`, async function () {
        const got = await router.getAmountOut(AMOUNT_IN, RESERVE, RESERVE, feeBps);
        const want = expectedAmountOut(AMOUNT_IN, RESERVE, RESERVE, feeBps);
        expect(got).to.equal(want);
      });
    }

    it("higher fee tier yields strictly less output for the same reserves", async function () {
      const tiers = [1, 5, 30, 100];
      const outputs = await Promise.all(
        tiers.map((f) => router.getAmountOut(AMOUNT_IN, RESERVE, RESERVE, f))
      );
      for (let i = 1; i < outputs.length; i++) {
        expect(outputs[i]).to.be.lessThan(outputs[i - 1],
          `feeBps=${tiers[i]} should yield less than feeBps=${tiers[i - 1]}`
        );
      }
    });
  });

  describe("createPair + fee-tier routing", function () {
    it("DEFAULT_FEE_BPS=30 auto-created on addLiquidity without explicit createPair", async function () {
      const DEFAULT_FEE_BPS = await router.DEFAULT_FEE_BPS();
      expect(DEFAULT_FEE_BPS).to.equal(30n);
      const feeBps = await router.getPairFeeBps(tokenA.target, tokenB.target);
      expect(feeBps).to.equal(30n);
    });

    it("explicit createPair sets 1 bps tier; swap yields more than 30 bps tier", async function () {
      const MockERC20 = await ethers.getContractFactory("MockERC20");
      const tokenC = await MockERC20.deploy("Token C", "TKC");
      const tokenD = await MockERC20.deploy("Token D", "TKD");

      const liq = ethers.parseEther("1000");
      await tokenC.mint(lp.address, liq);
      await tokenD.mint(lp.address, liq);
      await tokenC.connect(lp).approve(router.target, liq);
      await tokenD.connect(lp).approve(router.target, liq);

      await router.createPair(tokenC.target, tokenD.target, 1); // 0.01% tier
      await router.connect(lp).addLiquidity(tokenC.target, tokenD.target, liq, liq);

      const feeBps = await router.getPairFeeBps(tokenC.target, tokenD.target);
      expect(feeBps).to.equal(1n);

      const RESERVE = ethers.parseEther("1000");
      const AMOUNT_IN = ethers.parseEther("10");
      const out1bps = await router.getAmountOut(AMOUNT_IN, RESERVE, RESERVE, 1);
      const out30bps = await router.getAmountOut(AMOUNT_IN, RESERVE, RESERVE, 30);
      expect(out1bps).to.be.greaterThan(out30bps);
    });
  });

  describe("fee-denominator parity with CarbonCreditAMM", function () {
    // Both AMMs share the same denominator constant (10_000).
    // This test documents and enforces that invariant for AMMRouter.
    it("FEE_DENOMINATOR_BPS is 10_000", async function () {
      const denom = await router.FEE_DENOMINATOR_BPS();
      expect(denom).to.equal(10_000n,
        "AMMRouter and CarbonCreditAMM both use 10_000 as the fee denominator"
      );
    });

    it("fee calculation is numerically identical to CarbonCreditAMM formula", async function () {
      // CarbonCreditAMM formula (feeBasisPoints=30):
      //   amountInWithFee = amountIn * (10000 - 30)
      //   out = (amountInWithFee * reserveOut) / (reserveIn * 10000 + amountInWithFee)
      const amountIn = ethers.parseEther("50");
      const reserveIn = ethers.parseEther("500");
      const reserveOut = ethers.parseEther("800");
      const feeBps = 30;

      const routerOut = await router.getAmountOut(amountIn, reserveIn, reserveOut, feeBps);
      const referenceOut = expectedAmountOut(amountIn, reserveIn, reserveOut, feeBps);

      expect(routerOut).to.equal(referenceOut);
    });
  });
});
