// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IRevenueSharing
/// @notice Interface for routing protocol fee revenue to weather data contributors.
interface IRevenueSharing {
    event FeesFunded(address indexed funder, uint256 amount, uint256 undistributedFees);
    event DistributionPublished(
        uint256 indexed distributionId,
        bytes32 indexed merkleRoot,
        uint256 amount,
        bytes32 metadataHash
    );
    event DistributionUpdated(
        uint256 indexed distributionId,
        bytes32 indexed merkleRoot,
        uint256 amount,
        bytes32 metadataHash
    );
    event Claimed(
        uint256 indexed distributionId,
        uint256 indexed index,
        address indexed account,
        uint256 amount
    );

    function fund(uint256 amount) external;

    function publishDistribution(
        bytes32 merkleRoot,
        uint256 amount,
        bytes32 metadataHash
    ) external returns (uint256 distributionId);

    function updateDistributionRoot(
        uint256 distributionId,
        bytes32 merkleRoot,
        uint256 amount,
        bytes32 metadataHash
    ) external;

    function claim(
        uint256 distributionId,
        uint256 index,
        address account,
        uint256 amount,
        bytes32[] calldata merkleProof
    ) external;

    function isClaimed(uint256 distributionId, uint256 index) external view returns (bool);
}
