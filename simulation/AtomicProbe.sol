// SPDX-License-Identifier: MIT
pragma solidity 0.8.35;

struct PoolKey { address currency0; address currency1; uint24 fee; int24 tickSpacing; address hooks; }
struct SwapParams { bool zeroForOne; int256 amountSpecified; uint160 sqrtPriceLimitX96; }
interface IManager {
    function unlock(bytes calldata data) external returns (bytes memory);
    function swap(PoolKey calldata key, SwapParams calldata params, bytes calldata hookData) external returns (int256);
    function settle() external payable returns (uint256);
    function take(address currency, address to, uint256 amount) external;
    function extsload(bytes32 slot) external view returns (bytes32);
}
interface IERC20 { function balanceOf(address who) external view returns (uint256); }

/// Simulation-only code override. Never deploy or use to custody funds.
contract AtomicProbe {
    IManager constant MANAGER = IManager(0x8366a39CC670B4001A1121B8F6A443A643e40951);
    uint160 constant MIN_PRICE = 4295128739 + 1;
    uint160 constant MAX_PRICE = 1461446703485210103287273052203988822378723970342 - 1;
    receive() external payable { require(msg.sender == address(MANAGER)); }

    function execute(PoolKey calldata first, PoolKey calldata second, uint256 amount, bool failSecond)
        external payable returns (uint256 middle, uint256 output)
    {
        require(block.chainid == 4663 && msg.value == amount && amount > 0 && amount <= uint256(int256(type(int128).max)));
        require(first.currency0 == address(0) && second.currency0 == address(0));
        require(first.currency1 == second.currency1 && first.currency1 != address(0));
        require(keccak256(abi.encode(first)) != keccak256(abi.encode(second)));
        (middle, output) = abi.decode(MANAGER.unlock(abi.encode(first, second, amount, failSecond)), (uint256, uint256));
        (bool sent,) = msg.sender.call{value: output}("");
        require(sent);
    }

    function unlockCallback(bytes calldata data) external returns (bytes memory) {
        require(msg.sender == address(MANAGER));
        (PoolKey memory first, PoolKey memory second, uint256 amount, bool failSecond) = abi.decode(data, (PoolKey, PoolKey, uint256, bool));
        int256 firstDelta = MANAGER.swap(first, SwapParams(true, -int256(amount), MIN_PRICE), "");
        int128 firstInput = int128(firstDelta >> 128);
        int128 firstOutput = int128(firstDelta);
        require(firstInput == -int128(int256(amount)) && firstOutput > 0);
        uint256 middle = uint256(uint128(firstOutput));
        int256 secondDelta = MANAGER.swap(second, SwapParams(false, -int256(middle), failSecond ? 0 : MAX_PRICE), "");
        int128 secondInput = int128(secondDelta);
        int128 secondOutput = int128(secondDelta >> 128);
        require(secondInput == -firstOutput && secondOutput >= 0);
        uint256 output = uint256(uint128(secondOutput));
        MANAGER.settle{value: amount}();
        MANAGER.take(address(0), address(this), output);
        return abi.encode(middle, output);
    }

    function inspect(address caller, address token, bytes32 firstId, bytes32 secondId)
        external view returns (uint256 callerNative, uint256 callerToken, uint256 helperNative, uint256 helperToken, bytes32 firstSlot, bytes32 secondSlot)
    {
        return (caller.balance, IERC20(token).balanceOf(caller), address(this).balance, IERC20(token).balanceOf(address(this)),
            MANAGER.extsload(keccak256(abi.encode(firstId, uint256(6)))), MANAGER.extsload(keccak256(abi.encode(secondId, uint256(6)))));
    }
}
