#include "OscBridge.hpp"
#include "RackAgent.hpp"
#include "RackControl.hpp"
#include "plugin.hpp"

#include <cstdio>

struct Bridge8 : Module {
    enum InputIds { ENUMS(SEND_INPUT, controlroom::kLaneCount), NUM_INPUTS };
    enum OutputIds { ENUMS(RETURN_OUTPUT, controlroom::kLaneCount), NUM_OUTPUTS };
    enum ParamIds { NUM_PARAMS };
    enum LightIds { NUM_LIGHTS };

    controlroom::BridgeState bridgeState;
    controlroom::OscBridgeWorker worker{bridgeState};
    controlroom::RackAgentServer agentServer;
    dsp::ClockDivider transmitDivider;

    Bridge8() {
        config(NUM_PARAMS, NUM_INPUTS, NUM_OUTPUTS, NUM_LIGHTS);
        for (std::size_t lane = 0; lane < controlroom::kLaneCount; ++lane) {
            configInput(SEND_INPUT + lane, string::f("Send %d (0–10 V)", static_cast<int>(lane + 1)));
            configOutput(RETURN_OUTPUT + lane, string::f("Return %d (0–10 V)", static_cast<int>(lane + 1)));
        }
        transmitDivider.setDivision(64);
        INFO("ControlRoomBridge Bridge8 started");
    }

    ~Bridge8() override {
        INFO("ControlRoomBridge Bridge8 stopped: sent=%llu received=%llu errors=%llu",
             static_cast<unsigned long long>(bridgeState.sent.load(std::memory_order_relaxed)),
             static_cast<unsigned long long>(bridgeState.received.load(std::memory_order_relaxed)),
             static_cast<unsigned long long>(bridgeState.errors.load(std::memory_order_relaxed)));
    }

    void process(const ProcessArgs&) override {
        if (transmitDivider.process()) {
            for (std::size_t lane = 0; lane < controlroom::kLaneCount; ++lane) {
                const float normalized = clamp(inputs[SEND_INPUT + lane].getVoltage() / 10.f, 0.f, 1.f);
                bridgeState.rackToControl[lane].store(normalized, std::memory_order_relaxed);
            }
        }
        for (std::size_t lane = 0; lane < controlroom::kLaneCount; ++lane) {
            outputs[RETURN_OUTPUT + lane].setVoltage(
                bridgeState.controlToRack[lane].load(std::memory_order_relaxed) * 10.f);
        }
    }
};

struct Bridge8Overlay : widget::TransparentWidget {
    Bridge8* module = nullptr;

    void text(const DrawArgs& args, const std::shared_ptr<window::Font>& font, float x, float y, float size,
              NVGcolor color, const char* value, float spacing = 0.f) {
        nvgFontFaceId(args.vg, font->handle);
        nvgFontSize(args.vg, size);
        nvgTextLetterSpacing(args.vg, spacing);
        nvgTextAlign(args.vg, NVG_ALIGN_CENTER | NVG_ALIGN_MIDDLE);
        nvgFillColor(args.vg, color);
        const Vec point = mm2px(Vec(x, y));
        nvgText(args.vg, point.x, point.y, value, nullptr);
    }

    void draw(const DrawArgs& args) override {
        const auto font = APP->window->loadFont(asset::system("res/fonts/ShareTechMono-Regular.ttf"));
        if (!font) return;

        text(args, font, 20.f, 7.2f, 10.f, nvgRGB(80, 229, 255), "CONTROL ROOM", 1.2f);
        text(args, font, 20.f, 11.2f, 7.f, nvgRGB(126, 146, 165), "BRIDGE 8 // LOCAL OSC", 0.5f);
        text(args, font, 10.f, 17.1f, 8.f, nvgRGB(255, 174, 64), "SEND");
        text(args, font, 30.f, 17.1f, 8.f, nvgRGB(67, 230, 173), "RETURN");

        for (std::size_t lane = 0; lane < controlroom::kLaneCount; ++lane) {
            char number[2] = {static_cast<char>('1' + lane), '\0'};
            text(args, font, 20.f, 24.f + static_cast<float>(lane) * 12.f, 7.f, nvgRGB(105, 125, 143),
                 number);
        }

        char status[48] = {};
        const auto sent = module ? module->bridgeState.sent.load(std::memory_order_relaxed) : 0;
        const auto received = module ? module->bridgeState.received.load(std::memory_order_relaxed) : 0;
        std::snprintf(status, sizeof(status), "TX %llu  RX %llu", static_cast<unsigned long long>(sent),
                      static_cast<unsigned long long>(received));
        text(args, font, 20.f, 120.3f, 6.5f, nvgRGB(105, 125, 143), status, 0.35f);
        text(args, font, 20.f, 124.f, 6.5f, nvgRGB(126, 146, 165), "0-10V  <->  0-1", 0.35f);
    }
};

struct Bridge8Widget : ModuleWidget {
    Bridge8* bridge = nullptr;
    int lastCopilotHistoryIndex = -1;

    explicit Bridge8Widget(Bridge8* module) {
        setModule(module);
        bridge = module;
        setPanel(createPanel(asset::plugin(pluginInstance, "res/Bridge8.svg")));
        addChild(createWidget<ScrewSilver>(Vec(RACK_GRID_WIDTH, 0)));
        addChild(createWidget<ScrewSilver>(Vec(box.size.x - 2 * RACK_GRID_WIDTH, 0)));
        addChild(createWidget<ScrewSilver>(Vec(RACK_GRID_WIDTH, RACK_GRID_HEIGHT - RACK_GRID_WIDTH)));
        addChild(createWidget<ScrewSilver>(Vec(box.size.x - 2 * RACK_GRID_WIDTH, RACK_GRID_HEIGHT - RACK_GRID_WIDTH)));
        auto* overlay = createWidget<Bridge8Overlay>(Vec(0.f, 0.f));
        overlay->box.size = box.size;
        overlay->module = module;
        addChild(overlay);
        for (std::size_t lane = 0; lane < controlroom::kLaneCount; ++lane) {
            const float y = 24.f + static_cast<float>(lane) * 12.f;
            addInput(createInputCentered<PJ301MPort>(mm2px(Vec(10.f, y)), module, Bridge8::SEND_INPUT + lane));
            addOutput(createOutputCentered<PJ301MPort>(mm2px(Vec(30.f, y)), module, Bridge8::RETURN_OUTPUT + lane));
        }
    }

    void step() override {
        ModuleWidget::step();
        if (!bridge) return;
        auto request = bridge->agentServer.takeRequest();
        if (!request) return;
        bridge->agentServer.complete(
            request,
            controlroom::handleRackAgentRequest(request->payload, lastCopilotHistoryIndex));
    }
};

Model* modelBridge8 = createModel<Bridge8, Bridge8Widget>("Bridge8");
