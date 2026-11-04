#include "../src/OscBridge.hpp"

#include <cassert>
#include <cmath>
#include <cstring>

int main() {
    const auto outgoing = controlroom::encodeLane(3, 0.625f);
    assert(outgoing.size > 0);
    assert(std::strcmp(reinterpret_cast<const char*>(outgoing.bytes.data()), "/controlroom/in/3") == 0);
    assert(controlroom::encodeLane(0, 0.5f).size == 0);
    assert(controlroom::encodeLane(9, 0.5f).size == 0);

    auto incoming = outgoing;
    std::memcpy(incoming.bytes.data(), "/controlroom/out/3", std::strlen("/controlroom/out/3"));
    std::size_t lane = 99;
    float value = -1.f;
    assert(controlroom::decodeLane(incoming.bytes.data(), incoming.size, lane, value));
    assert(lane == 2);
    assert(std::fabs(value - 0.625f) < 0.00001f);

    incoming.bytes[0] = '!';
    assert(!controlroom::decodeLane(incoming.bytes.data(), incoming.size, lane, value));
    return 0;
}
