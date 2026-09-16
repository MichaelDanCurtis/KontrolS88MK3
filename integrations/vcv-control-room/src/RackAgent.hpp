#pragma once

#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <memory>
#include <mutex>
#include <string>
#include <thread>

namespace controlroom {

constexpr std::uint16_t kRackAgentPort = 9102;

struct AgentRequest {
    std::string payload;
    std::string response;
    bool claimed = false;
    bool completed = false;
};

/// Localhost TCP server that transports JSON between Control Room and Rack.
/// Rack UI work is deliberately not performed on this network thread.
class RackAgentServer {
  public:
    RackAgentServer();
    ~RackAgentServer();
    RackAgentServer(const RackAgentServer&) = delete;
    RackAgentServer& operator=(const RackAgentServer&) = delete;

    std::shared_ptr<AgentRequest> takeRequest();
    void complete(const std::shared_ptr<AgentRequest>& request, std::string response);
    std::uint64_t requests() const { return requests_.load(std::memory_order_relaxed); }
    std::uint64_t errors() const { return errors_.load(std::memory_order_relaxed); }

  private:
    void run();
    bool dispatch(std::string payload, std::string& response);

    std::atomic<bool> running_{true};
    std::atomic<std::uint64_t> requests_{0};
    std::atomic<std::uint64_t> errors_{0};
    std::mutex mutex_;
    std::condition_variable changed_;
    std::shared_ptr<AgentRequest> pending_;
    std::thread thread_;
};

}  // namespace controlroom
