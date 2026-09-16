#pragma once

#include <string>

namespace controlroom {

/// Parse and execute one agent request on Rack's UI thread.
std::string handleRackAgentRequest(const std::string& payload, int& lastCopilotHistoryIndex);

}  // namespace controlroom
