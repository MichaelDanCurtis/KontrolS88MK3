#include "RackAgent.hpp"

#include <array>
#include <algorithm>
#include <chrono>
#include <cstring>
#include <cerrno>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
using AgentSocket = SOCKET;
constexpr AgentSocket kInvalidAgentSocket = INVALID_SOCKET;
static void closeAgentSocket(AgentSocket socket) { closesocket(socket); }
#else
#include <arpa/inet.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>
using AgentSocket = int;
constexpr AgentSocket kInvalidAgentSocket = -1;
static void closeAgentSocket(AgentSocket socket) { close(socket); }
#endif

namespace controlroom {
namespace {

constexpr std::size_t kMaxMessageBytes = 4 * 1024 * 1024;

void setAgentNonBlocking(AgentSocket socket) {
#ifdef _WIN32
    u_long enabled = 1;
    ioctlsocket(socket, FIONBIO, &enabled);
#else
    const int flags = fcntl(socket, F_GETFL, 0);
    if (flags >= 0) fcntl(socket, F_SETFL, flags | O_NONBLOCK);
#endif
}

bool wouldBlock() {
#ifdef _WIN32
    const int error = WSAGetLastError();
    return error == WSAEWOULDBLOCK;
#else
    return errno == EAGAIN || errno == EWOULDBLOCK;
#endif
}

bool sendAll(AgentSocket socket, const std::string& value) {
    std::size_t offset = 0;
    while (offset < value.size()) {
        const int sent = send(socket, value.data() + offset, static_cast<int>(value.size() - offset), 0);
        if (sent <= 0) return false;
        offset += static_cast<std::size_t>(sent);
    }
    return true;
}

}  // namespace

RackAgentServer::RackAgentServer() : thread_([this] { run(); }) {}

RackAgentServer::~RackAgentServer() {
    running_.store(false, std::memory_order_release);
    changed_.notify_all();
    if (thread_.joinable()) thread_.join();
}

std::shared_ptr<AgentRequest> RackAgentServer::takeRequest() {
    std::lock_guard<std::mutex> lock(mutex_);
    if (!pending_ || pending_->claimed) return {};
    pending_->claimed = true;
    return pending_;
}

void RackAgentServer::complete(const std::shared_ptr<AgentRequest>& request, std::string response) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (!pending_ || pending_ != request) return;
    request->response = std::move(response);
    request->completed = true;
    changed_.notify_all();
}

bool RackAgentServer::dispatch(std::string payload, std::string& response) {
    auto request = std::make_shared<AgentRequest>();
    request->payload = std::move(payload);
    {
        std::unique_lock<std::mutex> lock(mutex_);
        if (pending_) return false;
        pending_ = request;
        changed_.notify_all();
        const bool completed = changed_.wait_for(lock, std::chrono::seconds(10), [&] {
            return request->completed || !running_.load(std::memory_order_acquire);
        });
        if (!completed || !request->completed) {
            pending_.reset();
            return false;
        }
        response = std::move(request->response);
        pending_.reset();
    }
    return true;
}

void RackAgentServer::run() {
#ifdef _WIN32
    WSADATA data{};
    if (WSAStartup(MAKEWORD(2, 2), &data) != 0) {
        errors_.fetch_add(1, std::memory_order_relaxed);
        return;
    }
#endif
    const AgentSocket listener = ::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (listener == kInvalidAgentSocket) {
        errors_.fetch_add(1, std::memory_order_relaxed);
#ifdef _WIN32
        WSACleanup();
#endif
        return;
    }
    int reuse = 1;
    setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, reinterpret_cast<const char*>(&reuse), sizeof(reuse));
    sockaddr_in local{};
    local.sin_family = AF_INET;
    local.sin_port = htons(kRackAgentPort);
    local.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (::bind(listener, reinterpret_cast<const sockaddr*>(&local), sizeof(local)) != 0 ||
        ::listen(listener, 4) != 0) {
        errors_.fetch_add(1, std::memory_order_relaxed);
        closeAgentSocket(listener);
#ifdef _WIN32
        WSACleanup();
#endif
        return;
    }
    setAgentNonBlocking(listener);

    while (running_.load(std::memory_order_acquire)) {
        const AgentSocket client = accept(listener, nullptr, nullptr);
        if (client == kInvalidAgentSocket) {
            if (!wouldBlock()) errors_.fetch_add(1, std::memory_order_relaxed);
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
            continue;
        }
        std::string request;
        std::array<char, 8192> chunk{};
        bool complete = false;
        while (request.size() <= kMaxMessageBytes) {
            const int count = recv(client, chunk.data(), static_cast<int>(chunk.size()), 0);
            if (count <= 0) break;
            const auto end = std::find(chunk.begin(), chunk.begin() + count, '\n');
            request.append(chunk.data(), static_cast<std::size_t>(end - chunk.begin()));
            if (end != chunk.begin() + count) {
                complete = true;
                break;
            }
        }
        std::string response;
        if (!complete || request.size() > kMaxMessageBytes || !dispatch(std::move(request), response)) {
            response = R"({"ok":false,"error":"Rack UI did not accept the request"})";
            errors_.fetch_add(1, std::memory_order_relaxed);
        } else {
            requests_.fetch_add(1, std::memory_order_relaxed);
        }
        response.push_back('\n');
        if (!sendAll(client, response)) errors_.fetch_add(1, std::memory_order_relaxed);
        closeAgentSocket(client);
    }
    closeAgentSocket(listener);
#ifdef _WIN32
    WSACleanup();
#endif
}

}  // namespace controlroom
