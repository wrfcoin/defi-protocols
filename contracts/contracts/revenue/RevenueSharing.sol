// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {Ownable2Step, Ownable} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IRevenueSharing} from "../interfaces/IRevenueSharing.sol";

/// @title RevenueSharing
/// @notice Distributes protocol fee revenue to weather data contributors using Merkle proofs.
/// @dev Fee tokens are funded into the contract first, then reserved into per-epoch Merkle
///      distributions. Each contributor can claim once per distribution index.
contract RevenueSharing is
    ReentrancyGuard,
    Ownable2Step,
    IRevenueSharing
{
    using SafeERC20 for IERC20;

    struct Distribution {
        bytes32 merkleRoot;
        uint256 amount;
        uint256 claimedAmount;
        bytes32 metadataHash;
    }

    IERC20 public immutable feeToken;
    uint256 public undistributedFees;
    uint256 public distributionCount;

    mapping(uint256 distributionId => Distribution distribution) public distributions;
    mapping(uint256 distributionId => mapping(uint256 wordIndex => uint256 claimedWord))
        private _claimedBitMap;

    error ZeroAmount();
    error ZeroMerkleRoot();
    error InvalidDistribution();
    error InsufficientUndistributedFees();
    error DistributionLocked();
    error AlreadyClaimed();
    error InvalidProof();
    error DistributionExhausted();

    constructor(IERC20 _feeToken, address _owner) Ownable(_owner) {
        feeToken = _feeToken;
    }

    /// @notice Fund the contract with protocol fees that can later be reserved into a distribution.
    function fund(uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();

        feeToken.safeTransferFrom(msg.sender, address(this), amount);
        undistributedFees += amount;

        emit FeesFunded(msg.sender, amount, undistributedFees);
    }

    /// @notice Publish a new contributor revenue distribution against funded fees.
    function publishDistribution(
        bytes32 merkleRoot,
        uint256 amount,
        bytes32 metadataHash
    ) external onlyOwner returns (uint256 distributionId) {
        if (merkleRoot == bytes32(0)) revert ZeroMerkleRoot();
        if (amount == 0) revert ZeroAmount();
        if (amount > undistributedFees) revert InsufficientUndistributedFees();

        distributionId = distributionCount;
        distributionCount += 1;
        undistributedFees -= amount;

        distributions[distributionId] = Distribution({
            merkleRoot: merkleRoot,
            amount: amount,
            claimedAmount: 0,
            metadataHash: metadataHash
        });

        emit DistributionPublished(distributionId, merkleRoot, amount, metadataHash);
    }

    /// @notice Update the Merkle root for an unpublished-claim distribution, preserving fee safety.
    function updateDistributionRoot(
        uint256 distributionId,
        bytes32 merkleRoot,
        uint256 amount,
        bytes32 metadataHash
    ) external onlyOwner {
        if (merkleRoot == bytes32(0)) revert ZeroMerkleRoot();
        if (amount == 0) revert ZeroAmount();

        Distribution storage distribution = distributions[distributionId];
        if (distribution.merkleRoot == bytes32(0)) revert InvalidDistribution();
        if (distribution.claimedAmount != 0) revert DistributionLocked();

        uint256 previousAmount = distribution.amount;
        if (amount > previousAmount) {
            uint256 additionalAmount = amount - previousAmount;
            if (additionalAmount > undistributedFees) revert InsufficientUndistributedFees();
            undistributedFees -= additionalAmount;
        } else {
            unchecked {
                undistributedFees += previousAmount - amount;
            }
        }

        distribution.merkleRoot = merkleRoot;
        distribution.amount = amount;
        distribution.metadataHash = metadataHash;

        emit DistributionUpdated(distributionId, merkleRoot, amount, metadataHash);
    }

    /// @notice Claim contributor revenue for an account in a specific distribution.
    /// @dev Anyone can submit the proof, but the payout always goes to the encoded `account`.
    function claim(
        uint256 distributionId,
        uint256 index,
        address account,
        uint256 amount,
        bytes32[] calldata merkleProof
    ) external nonReentrant {
        Distribution storage distribution = distributions[distributionId];
        if (distribution.merkleRoot == bytes32(0)) revert InvalidDistribution();
        if (_isClaimed(distributionId, index)) revert AlreadyClaimed();

        bytes32 leaf = keccak256(bytes.concat(keccak256(abi.encode(index, account, amount))));
        if (!MerkleProof.verifyCalldata(merkleProof, distribution.merkleRoot, leaf)) {
            revert InvalidProof();
        }

        uint256 updatedClaimedAmount = distribution.claimedAmount + amount;
        if (updatedClaimedAmount > distribution.amount) revert DistributionExhausted();

        _setClaimed(distributionId, index);
        distribution.claimedAmount = updatedClaimedAmount;
        feeToken.safeTransfer(account, amount);

        emit Claimed(distributionId, index, account, amount);
    }

    function isClaimed(uint256 distributionId, uint256 index) external view returns (bool) {
        return _isClaimed(distributionId, index);
    }

    function _isClaimed(uint256 distributionId, uint256 index) internal view returns (bool) {
        uint256 claimedWord = _claimedBitMap[distributionId][index / 256];
        uint256 mask = 1 << (index % 256);
        return claimedWord & mask == mask;
    }

    function _setClaimed(uint256 distributionId, uint256 index) internal {
        _claimedBitMap[distributionId][index / 256] |= 1 << (index % 256);
    }
}
