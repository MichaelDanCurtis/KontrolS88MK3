#include "OscBridge.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstring>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
using SocketHandle = SOCKET;
constexpr SocketHandle kInvalidSocket = INVALID_SOCKET;
static void closeSocket(SocketHandle socket) { closesocket(socket); }
#else
#include <arpa/inet.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>
using SocketHandle = int;
constexpr SocketHandle kInvalidSocket = -1;
static void closeSocket(SocketHandle socket) { close(socket); }
#endif

namespace controlroom {
namespace {

std::size_t paddedStringSize(std::size_t length) { return (length + 1 + 3) & ~std::size_t(3); }

float clampNormalized(float value) { return std::max(0.f, std::min(1.f, value)); }

bool readPaddedString(const std::uint8_t* bytes, std::size_t size, std::size_t& offset, const char*& value) {
    if (offset >= size) return false;
    value = reinterpret_cast<const char*>(bytes + offset);
    const void* end = std::memchr(value, '\0', size - offset);
    if (!end) return false;
    const auto length = static_cast<const char*>(end) - value;
    offset += paddedStringSize(static_cast<std::size_t>(length));
    return offset <= size;
}

void setNonBlocking(SocketHandle socket) {
#ifdef _WIN32
    u_long enabled = 1;
    ioctlsocket(socket, FIONBIO, &enabled);
#else
    const int flags = fcntl(socket, F_GETFL, 0);
    if (flags >= 0) fcntl(socket, F_SETFL, flags | O_NONBLOCK);
#endif
}

}  // namespace

OscPacket encodeLane(std::size_t oneBasedLane, float normalized) {
    OscPacket packet;
    if (oneBasedLane == 0 || oneBasedLane > kLaneCount || !std::isfinite(normalized)) return packet;
    const std::string address = "/controlroom/in/" + std::to_string(oneBasedLane);
    const std::string typeTag = ",f";
    const auto addressSize = paddedStringSize(address.size());
    const auto typeSize = paddedStringSize(typeTag.size());
    packet.size = addressSize + typeSize + 4;
    if (packet.size > packet.bytes.size()) {
        packet.size = 0;
        return packet;
    }
    std::memcpy(packet.bytes.data(), address.c_str(), address.size());
    std::memcpy(packet.bytes.data() + addressSize, typeTag.c_str(), typeTag.size());
    float value = clampNormalized(normalized);
    std::uint32_t bits = 0;
    std::memcpy(&bits, &value, sizeof(bits));
    bits = htonl(bits);
    std::memcpy(packet.bytes.data() + addressSize + typeSize, &bits, sizeof(bits));
    return packet;
}

bool decodeLane(const std::uint8_t* bytes, std::size_t size, std::size_t& zeroBasedLane, float& normalized) {
    if (!bytes || size < 12) return false;
    std::size_t offset = 0;
    const char* address = nullptr;
    const char* typeTag = nullptr;
    if (!readPaddedString(bytes, size, offset, address) || !readPaddedString(bytes, size, offset, typeTag)) return false;
    constexpr const char* prefix = "/controlroom/out/";
    const auto prefixLength = std::strlen(prefix);
    if (std::strncmp(address, prefix, prefixLength) != 0 || std::strcmp(typeTag, ",f") != 0) return false;
    const char laneCharacter = address[prefixLength];
    if (laneCharacter < '1' || laneCharacter > '8' || address[prefixLength + 1] != '\0') return false;
    if (offset + 4 != size) return false;
    std::uint32_t bits = 0;
    std::memcpy(&bits, bytes + offset, sizeof(bits));
    bits = ntohl(bits);
    float value = 0.f;
    std::memcpy(&value, &bits, sizeof(value));
    if (!std::isfinite(value)) return false;
    zeroBasedLane = static_cast<std::size_t>(laneCharacter - '1');
    normalized = clampNormalized(value);
    return true;
}

OscBridgeWorker::OscBridgeWorker(BridgeState& state) : state_(state), thread_([this] { run(); }) {}

OscBridgeWorker::~OscBridgeWorker() {
    running_.store(false, std::memory_order_release);
    if (thread_.joinable()) thread_.join();
}

void OscBridgeWorker::run() {
#ifdef _WIN32
    WSADATA data{};
    if (WSAStartup(MAKEWORD(2, 2), &data) != 0) {
        state_.errors.fetch_add(1, std::memory_order_relaxed);
        return;
    }
#endif
    const SocketHandle socket = ::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (socket == kInvalidSocket) {
        state_.errors.fetch_add(1, std::memory_order_relaxed);
#ifdef _WIN32
        WSACleanup();
#endif
        return;
    }
    sockaddr_in local{};
    local.sin_family = AF_INET;
    local.sin_port = htons(kRackPort);
    local.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (::bind(socket, reinterpret_cast<const sockaddr*>(&local), sizeof(local)) != 0) {
        state_.errors.fetch_add(1, std::memory_order_relaxed);
        closeSocket(socket);
#ifdef _WIN32
        WSACleanup();
#endif
        return;
    }
    setNonBlocking(socket);
    sockaddr_in peer{};
    peer.sin_family = AF_INET;
    peer.sin_port = htons(kControlRoomPort);
    peer.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    std::array<float, kLaneCount> previous{};
    previous.fill(-100.f);
    std::array<std::uint8_t, 256> incoming{};

    while (running_.load(std::memory_order_acquire)) {
        for (std::size_t lane = 0; lane < kLaneCount; ++lane) {
            const float value = state_.rackToControl[lane].load(std::memory_order_relaxed);
            if (std::fabs(value - previous[lane]) < 0.0005f) continue;
            const auto packet = encodeLane(lane + 1, value);
            if (packet.size > 0 && sendto(socket, reinterpret_cast<const char*>(packet.bytes.data()), static_cast<int>(packet.size), 0, reinterpret_cast<const sockaddr*>(&peer), sizeof(peer)) >= 0) {
                previous[lane] = value;
                state_.sent.fetch_add(1, std::memory_order_relaxed);
            } else {
                state_.errors.fetch_add(1, std::memory_order_relaxed);
            }
        }
        for (;;) {
            const int received = recvfrom(socket, reinterpret_cast<char*>(incoming.data()), static_cast<int>(incoming.size()), 0, nullptr, nullptr);
            if (received <= 0) break;
            std::size_t lane = 0;
            float value = 0.f;
            if (decodeLane(incoming.data(), static_cast<std::size_t>(received), lane, value)) {
                state_.controlToRack[lane].store(value, std::memory_order_relaxed);
                state_.received.fetch_add(1, std::memory_order_relaxed);
            } else {
                state_.errors.fetch_add(1, std::memory_order_relaxed);
            }
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    closeSocket(socket);
#ifdef _WIN32
    WSACleanup();
#endif
}

}  // namespace controlroom
