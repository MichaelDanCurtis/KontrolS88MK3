#pragma once

#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <thread>

namespace controlroom {

constexpr std::size_t kLaneCount = 8;
constexpr std::uint16_t kControlRoomPort = 9100;
constexpr std::uint16_t kRackPort = 9101;

struct OscPacket {
    std::array<std::uint8_t, 64> bytes{};
    std::size_t size = 0;
};

OscPacket encodeLane(std::size_t oneBasedLane, float normalized);
bool decodeLane(const std::uint8_t* bytes, std::size_t size, std::size_t& zeroBasedLane, float& normalized);

struct BridgeState {
    std::array<std::atomic<float>, kLaneCount> rackToControl{};
    std::array<std::atomic<float>, kLaneCount> controlToRack{};
    std::atomic<std::uint64_t> sent{0};
    std::atomic<std::uint64_t> received{0};
    std::atomic<std::uint64_t> errors{0};
};

class OscBridgeWorker {
  public:
    explicit OscBridgeWorker(BridgeState& state);
    ~OscBridgeWorker();
    OscBridgeWorker(const OscBridgeWorker&) = delete;
    OscBridgeWorker& operator=(const OscBridgeWorker&) = delete;

  private:
    void run();
    BridgeState& state_;
    std::atomic<bool> running_{true};
    std::thread thread_;
};

}  // namespace controlroom
