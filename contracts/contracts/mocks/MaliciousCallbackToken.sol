// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract MaliciousCallbackToken is ERC20 {
    address public trustedCaller;
    address public reentryTarget;
    bytes public reentryData;
    bool public reentryEnabled;
    bool private _reentered;

    constructor(string memory name_, string memory symbol_) ERC20(name_, symbol_) {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function configureReentrancy(
        address trustedCaller_,
        address reentryTarget_,
        bytes calldata reentryData_
    ) external {
        trustedCaller = trustedCaller_;
        reentryTarget = reentryTarget_;
        reentryData = reentryData_;
        reentryEnabled = true;
        _reentered = false;
    }

    function disableReentrancy() external {
        reentryEnabled = false;
        _reentered = false;
    }

    function transfer(address to, uint256 value) public override returns (bool) {
        bool success = super.transfer(to, value);
        _maybeReenter();
        return success;
    }

    function transferFrom(address from, address to, uint256 value) public override returns (bool) {
        bool success = super.transferFrom(from, to, value);
        _maybeReenter();
        return success;
    }

    function _maybeReenter() internal {
        if (
            !reentryEnabled ||
            _reentered ||
            reentryTarget == address(0) ||
            msg.sender != trustedCaller
        ) {
            return;
        }

        _reentered = true;
        (bool success, bytes memory returndata) = reentryTarget.call(reentryData);
        if (!success) {
            assembly {
                revert(add(returndata, 0x20), mload(returndata))
            }
        }
        _reentered = false;
    }
}
