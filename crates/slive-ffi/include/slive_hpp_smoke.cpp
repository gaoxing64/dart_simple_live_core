#include "slive.hpp"
#include <cassert>
#include <iostream>

int main() {
    // 仅编译期/链接期验证：不实际 init（无 dll 可链）
    static_assert(std::is_move_constructible_v<slive::Site>, "Site 必须可移动构造");
    static_assert(std::is_move_assignable_v<slive::Site>, "Site 必须可移动赋值");
    static_assert(!std::is_copy_constructible_v<slive::Site>, "Site 不应可拷贝");
    static_assert(std::is_move_constructible_v<slive::Danmaku>);
    static_assert(std::is_move_assignable_v<slive::Danmaku>);
    static_assert(std::is_move_constructible_v<slive::Mask>);
    static_assert(std::is_move_assignable_v<slive::Mask>);
    static_assert(!std::is_move_constructible_v<slive::Runtime>, "Runtime 持 mutex，不应可移动");
    static_assert(!std::is_copy_constructible_v<slive::Runtime>);

    // 枚举值与 C 头一致
    static_assert(static_cast<int32_t>(slive::Platform::Bilibili) == SLIVE_PLATFORM_BILIBILI);
    static_assert(static_cast<int32_t>(slive::Platform::Twitch) == SLIVE_PLATFORM_TWITCH);
    std::cout << "slive.hpp compiles\n";
    return 0;
}
