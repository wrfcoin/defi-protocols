'use strict';

const { expect } = require('chai');
const { ethers } = require('hardhat');
const { loadFixture } = require('@nomicfoundation/hardhat-network-helpers');

const abiCoder = ethers.AbiCoder.defaultAbiCoder();

function hashLeaf(index, account, amount) {
  const encoded = abiCoder.encode(['uint256', 'address', 'uint256'], [index, account, amount]);
  return ethers.keccak256(ethers.concat([ethers.keccak256(encoded)]));
}

function hashPair(left, right) {
  if (!right) {
    return left;
  }

  return BigInt(left) < BigInt(right)
    ? ethers.keccak256(ethers.concat([left, right]))
    : ethers.keccak256(ethers.concat([right, left]));
}

function buildMerkleTree(entries) {
  const leaves = entries.map(({ index, account, amount }) => hashLeaf(index, account, amount));
  const layers = [leaves];

  while (layers[layers.length - 1].length > 1) {
    const current = layers[layers.length - 1];
    const next = [];

    for (let i = 0; i < current.length; i += 2) {
      next.push(hashPair(current[i], current[i + 1]));
    }

    layers.push(next);
  }

  return {
    root: layers[layers.length - 1][0] ?? ethers.ZeroHash,
    proof(index) {
      const proof = [];
      let pointer = index;

      for (let layer = 0; layer < layers.length - 1; layer += 1) {
        const current = layers[layer];
        const siblingIndex = pointer ^ 1;
        if (siblingIndex < current.length) {
          proof.push(current[siblingIndex]);
        }
        pointer = Math.floor(pointer / 2);
      }

      return proof;
    },
  };
}

describe('RevenueSharing', function () {
  async function deployFixture() {
    const [owner, ops, contributorA, contributorB, contributorC, relayer] =
      await ethers.getSigners();

    const MockERC20 = await ethers.getContractFactory('MockERC20');
    const feeToken = await MockERC20.deploy('Protocol Fee', 'PFEE');

    const distributor = await (
      await ethers.getContractFactory('RevenueSharing')
    ).deploy(await feeToken.getAddress(), owner.address);

    await feeToken.mint(ops.address, ethers.parseEther('1000'));
    await feeToken.connect(ops).approve(await distributor.getAddress(), ethers.parseEther('1000'));

    return {
      owner,
      ops,
      contributorA,
      contributorB,
      contributorC,
      relayer,
      feeToken,
      distributor,
    };
  }

  it('funds fees and publishes a contributor distribution', async function () {
    const { owner, ops, contributorA, contributorB, distributor } = await loadFixture(deployFixture);
    const entries = [
      { index: 0, account: contributorA.address, amount: ethers.parseEther('12') },
      { index: 1, account: contributorB.address, amount: ethers.parseEther('8') },
    ];
    const tree = buildMerkleTree(entries);
    const metadataHash = ethers.id('weather-fees-epoch-1');

    await expect(distributor.connect(ops).fund(ethers.parseEther('40')))
      .to.emit(distributor, 'FeesFunded')
      .withArgs(ops.address, ethers.parseEther('40'), ethers.parseEther('40'));

    await expect(
      distributor
        .connect(owner)
        .publishDistribution(tree.root, ethers.parseEther('20'), metadataHash)
    )
      .to.emit(distributor, 'DistributionPublished')
      .withArgs(0, tree.root, ethers.parseEther('20'), metadataHash);

    const distribution = await distributor.distributions(0);
    expect(distribution.merkleRoot).to.equal(tree.root);
    expect(distribution.amount).to.equal(ethers.parseEther('20'));
    expect(distribution.claimedAmount).to.equal(0);
    expect(distribution.metadataHash).to.equal(metadataHash);
    expect(await distributor.undistributedFees()).to.equal(ethers.parseEther('20'));
  });

  it('allows a relayer to claim on behalf of a contributor via a valid proof', async function () {
    const { owner, ops, contributorA, contributorB, relayer, feeToken, distributor } =
      await loadFixture(deployFixture);
    const entries = [
      { index: 0, account: contributorA.address, amount: ethers.parseEther('12') },
      { index: 1, account: contributorB.address, amount: ethers.parseEther('8') },
    ];
    const tree = buildMerkleTree(entries);

    await distributor.connect(ops).fund(ethers.parseEther('20'));
    await distributor.connect(owner).publishDistribution(tree.root, ethers.parseEther('20'), ethers.id('epoch-1'));

    await expect(
      distributor
        .connect(relayer)
        .claim(0, 0, contributorA.address, entries[0].amount, tree.proof(0))
    )
      .to.emit(distributor, 'Claimed')
      .withArgs(0, 0, contributorA.address, entries[0].amount);

    expect(await feeToken.balanceOf(contributorA.address)).to.equal(entries[0].amount);
    expect(await distributor.isClaimed(0, 0)).to.equal(true);

    const distribution = await distributor.distributions(0);
    expect(distribution.claimedAmount).to.equal(entries[0].amount);
  });

  it('supports root updates before claims and rebalances undistributed fees', async function () {
    const { owner, ops, contributorA, contributorB, contributorC, distributor } =
      await loadFixture(deployFixture);
    const epochOne = buildMerkleTree([
      { index: 0, account: contributorA.address, amount: ethers.parseEther('10') },
      { index: 1, account: contributorB.address, amount: ethers.parseEther('10') },
    ]);
    const updatedEpoch = buildMerkleTree([
      { index: 0, account: contributorA.address, amount: ethers.parseEther('9') },
      { index: 1, account: contributorB.address, amount: ethers.parseEther('7') },
      { index: 2, account: contributorC.address, amount: ethers.parseEther('4') },
    ]);

    await distributor.connect(ops).fund(ethers.parseEther('40'));
    await distributor
      .connect(owner)
      .publishDistribution(epochOne.root, ethers.parseEther('20'), ethers.id('epoch-1'));

    await expect(
      distributor
        .connect(owner)
        .updateDistributionRoot(0, updatedEpoch.root, ethers.parseEther('20'), ethers.id('epoch-1-revised'))
    )
      .to.emit(distributor, 'DistributionUpdated')
      .withArgs(0, updatedEpoch.root, ethers.parseEther('20'), ethers.id('epoch-1-revised'));

    const distribution = await distributor.distributions(0);
    expect(distribution.merkleRoot).to.equal(updatedEpoch.root);
    expect(distribution.metadataHash).to.equal(ethers.id('epoch-1-revised'));
    expect(await distributor.undistributedFees()).to.equal(ethers.parseEther('20'));
  });

  it('rejects invalid proofs and double claims', async function () {
    const { owner, ops, contributorA, contributorB, contributorC, distributor } =
      await loadFixture(deployFixture);
    const entries = [
      { index: 0, account: contributorA.address, amount: ethers.parseEther('12') },
      { index: 1, account: contributorB.address, amount: ethers.parseEther('8') },
    ];
    const tree = buildMerkleTree(entries);

    await distributor.connect(ops).fund(ethers.parseEther('20'));
    await distributor.connect(owner).publishDistribution(tree.root, ethers.parseEther('20'), ethers.id('epoch-1'));

    await expect(
      distributor.claim(0, 0, contributorC.address, entries[0].amount, tree.proof(0))
    ).to.be.revertedWithCustomError(distributor, 'InvalidProof');

    await distributor.claim(0, 0, contributorA.address, entries[0].amount, tree.proof(0));

    await expect(
      distributor.claim(0, 0, contributorA.address, entries[0].amount, tree.proof(0))
    ).to.be.revertedWithCustomError(distributor, 'AlreadyClaimed');
  });

  it('rejects oversubscribed or locked distributions', async function () {
    const { owner, ops, contributorA, contributorB, distributor } = await loadFixture(deployFixture);
    const oversizedTree = buildMerkleTree([
      { index: 0, account: contributorA.address, amount: ethers.parseEther('15') },
      { index: 1, account: contributorB.address, amount: ethers.parseEther('10') },
    ]);

    await distributor.connect(ops).fund(ethers.parseEther('25'));
    await distributor
      .connect(owner)
      .publishDistribution(oversizedTree.root, ethers.parseEther('20'), ethers.id('epoch-2'));

    await distributor.claim(0, 0, contributorA.address, ethers.parseEther('15'), oversizedTree.proof(0));

    await expect(
      distributor.claim(0, 1, contributorB.address, ethers.parseEther('10'), oversizedTree.proof(1))
    ).to.be.revertedWithCustomError(distributor, 'DistributionExhausted');

    await expect(
      distributor
        .connect(owner)
        .updateDistributionRoot(0, oversizedTree.root, ethers.parseEther('20'), ethers.id('epoch-2-revised'))
    ).to.be.revertedWithCustomError(distributor, 'DistributionLocked');
  });

  it('rejects invalid publish and update operations', async function () {
    const { owner, ops, distributor } = await loadFixture(deployFixture);

    await expect(
      distributor.connect(owner).publishDistribution(ethers.ZeroHash, ethers.parseEther('1'), ethers.ZeroHash)
    ).to.be.revertedWithCustomError(distributor, 'ZeroMerkleRoot');

    await expect(
      distributor.connect(ops).fund(0)
    ).to.be.revertedWithCustomError(distributor, 'ZeroAmount');

    await distributor.connect(ops).fund(ethers.parseEther('5'));

    await expect(
      distributor.connect(owner).publishDistribution(ethers.id('root'), ethers.parseEther('6'), ethers.ZeroHash)
    ).to.be.revertedWithCustomError(distributor, 'InsufficientUndistributedFees');

    await distributor
      .connect(owner)
      .publishDistribution(ethers.id('root'), ethers.parseEther('5'), ethers.id('epoch-3'));

    await expect(
      distributor
        .connect(owner)
        .updateDistributionRoot(0, ethers.id('new-root'), ethers.parseEther('6'), ethers.ZeroHash)
    ).to.be.revertedWithCustomError(distributor, 'InsufficientUndistributedFees');

    await expect(
      distributor
        .connect(owner)
        .updateDistributionRoot(1, ethers.id('missing'), ethers.parseEther('1'), ethers.ZeroHash)
    ).to.be.revertedWithCustomError(distributor, 'InvalidDistribution');
  });
});
