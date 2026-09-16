#include "RackControl.hpp"

#include "plugin.hpp"
#include <tag.hpp>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <iomanip>
#include <map>
#include <memory>
#include <limits>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace controlroom {
namespace {

using Json = std::unique_ptr<json_t, decltype(&json_decref)>;

Json owned(json_t* value) { return Json(value, json_decref); }

std::string dump(json_t* value) {
    char* encoded = json_dumps(value, JSON_COMPACT | JSON_ENCODE_ANY);
    if (!encoded) return "{}";
    std::string result(encoded);
    free(encoded);
    return result;
}

std::string ok(json_t* body = nullptr) {
    Json root = owned(json_object());
    json_object_set_new(root.get(), "ok", json_true());
    if (body && json_is_object(body)) {
        const char* key = nullptr;
        json_t* value = nullptr;
        json_object_foreach(body, key, value) { json_object_set(root.get(), key, value); }
    }
    return dump(root.get());
}

std::string fail(const std::string& message) {
    Json root = owned(json_object());
    json_object_set_new(root.get(), "ok", json_false());
    json_object_set_new(root.get(), "error", json_string(message.c_str()));
    return dump(root.get());
}

std::string requiredString(json_t* root, const char* key) {
    json_t* value = json_object_get(root, key);
    if (!json_is_string(value) || std::string(json_string_value(value)).empty())
        throw std::runtime_error(std::string("missing string field ") + key);
    return json_string_value(value);
}

int requiredInt(json_t* root, const char* key) {
    json_t* value = json_object_get(root, key);
    if (!json_is_integer(value)) throw std::runtime_error(std::string("missing integer field ") + key);
    const json_int_t parsed = json_integer_value(value);
    if (parsed < std::numeric_limits<int>::min() || parsed > std::numeric_limits<int>::max())
        throw std::runtime_error(std::string("integer out of range: ") + key);
    return static_cast<int>(parsed);
}

float requiredFloat(json_t* root, const char* key) {
    json_t* value = json_object_get(root, key);
    if (!json_is_number(value)) throw std::runtime_error(std::string("missing numeric field ") + key);
    const double parsed = json_number_value(value);
    if (!std::isfinite(parsed)) throw std::runtime_error(std::string("non-finite field ") + key);
    return static_cast<float>(parsed);
}

std::string moduleReference(int64_t id) { return "rack:" + std::to_string(id); }

engine::Module* moduleForModel(plugin::Model* model, app::RackWidget* rack, bool& ownedModule) {
    for (app::ModuleWidget* widget : rack->getModules()) {
        if (widget && widget->model == model && widget->module) {
            ownedModule = false;
            return widget->module;
        }
    }
    if (model->plugin && model->plugin->slug == "ControlRoomBridge") {
        ownedModule = false;
        return nullptr;
    }
    ownedModule = true;
    return model->createModule();
}

json_t* portsJson(const std::vector<engine::PortInfo*>& infos) {
    json_t* ports = json_array();
    for (std::size_t id = 0; id < infos.size(); ++id) {
        json_t* port = json_object();
        json_object_set_new(port, "id", json_integer(static_cast<json_int_t>(id)));
        const std::string name = infos[id] && !infos[id]->name.empty()
                                     ? infos[id]->name
                                     : "Port " + std::to_string(id);
        json_object_set_new(port, "name", json_string(name.c_str()));
        json_array_append_new(ports, port);
    }
    return ports;
}

json_t* paramsJson(engine::Module* module, bool includeValues) {
    json_t* params = json_array();
    if (!module) return params;
    for (std::size_t id = 0; id < module->paramQuantities.size(); ++id) {
        engine::ParamQuantity* quantity = module->paramQuantities[id];
        if (!quantity) continue;
        json_t* param = json_object();
        json_object_set_new(param, "id", json_integer(static_cast<json_int_t>(id)));
        const std::string name = quantity->name.empty() ? "Parameter " + std::to_string(id) : quantity->name;
        json_object_set_new(param, "name", json_string(name.c_str()));
        if (includeValues) {
            json_object_set_new(param, "value", json_real(APP->engine->getParamValue(module, static_cast<int>(id))));
        } else {
            json_object_set_new(param, "min", json_real(quantity->minValue));
            json_object_set_new(param, "max", json_real(quantity->maxValue));
            json_object_set_new(param, "default", json_real(quantity->defaultValue));
        }
        json_array_append_new(params, param);
    }
    return params;
}

json_t* catalogJson(app::RackWidget* rack) {
    json_t* catalog = json_array();
    for (plugin::Plugin* plugin : plugin::plugins) {
        if (!plugin) continue;
        for (plugin::Model* model : plugin->models) {
            if (!model || model->hidden) continue;
            bool deleteModule = false;
            engine::Module* module = nullptr;
            try {
                module = moduleForModel(model, rack, deleteModule);
            } catch (const std::exception&) {
                module = nullptr;
            }
            json_t* entry = json_object();
            json_object_set_new(entry, "plugin", json_string(plugin->slug.c_str()));
            json_object_set_new(entry, "model", json_string(model->slug.c_str()));
            json_object_set_new(entry, "name", json_string(model->name.c_str()));
            json_object_set_new(entry, "description", json_string(model->description.c_str()));
            json_t* tags = json_array();
            for (int tagId : model->tagIds) {
                const std::string tagName = tag::getTag(tagId);
                json_array_append_new(tags, json_string(tagName.c_str()));
            }
            json_object_set_new(entry, "tags", tags);
            json_object_set_new(entry, "params", paramsJson(module, false));
            json_object_set_new(entry, "inputs", module ? portsJson(module->inputInfos) : json_array());
            json_object_set_new(entry, "outputs", module ? portsJson(module->outputInfos) : json_array());
            json_array_append_new(catalog, entry);
            if (deleteModule) delete module;
        }
    }
    return catalog;
}

json_t* patchJson(app::RackWidget* rack) {
    json_t* patch = json_object();
    json_t* modules = json_array();
    std::vector<app::ModuleWidget*> widgets = rack->getModules();
    std::sort(widgets.begin(), widgets.end(), [](app::ModuleWidget* left, app::ModuleWidget* right) {
        return left->module->id < right->module->id;
    });
    for (app::ModuleWidget* widget : widgets) {
        if (!widget || !widget->module || !widget->model || !widget->model->plugin) continue;
        json_t* module = json_object();
        json_object_set_new(module, "reference", json_string(moduleReference(widget->module->id).c_str()));
        json_object_set_new(module, "rack_id", json_integer(widget->module->id));
        json_object_set_new(module, "plugin", json_string(widget->model->plugin->slug.c_str()));
        json_object_set_new(module, "model", json_string(widget->model->slug.c_str()));
        json_object_set_new(module, "name", json_string(widget->model->name.c_str()));
        json_object_set_new(module, "x", json_real(widget->box.pos.x / RACK_GRID_WIDTH));
        json_object_set_new(module, "y", json_real(widget->box.pos.y / RACK_GRID_HEIGHT));
        json_object_set_new(module, "params", paramsJson(widget->module, true));
        json_array_append_new(modules, module);
    }
    json_object_set_new(patch, "modules", modules);

    std::vector<app::CableWidget*> cableWidgets = rack->getCompleteCables();
    std::sort(cableWidgets.begin(), cableWidgets.end(), [](app::CableWidget* left, app::CableWidget* right) {
        return left->cable->id < right->cable->id;
    });
    json_t* cables = json_array();
    for (app::CableWidget* widget : cableWidgets) {
        engine::Cable* cable = widget ? widget->getCable() : nullptr;
        if (!cable || !cable->outputModule || !cable->inputModule) continue;
        json_t* entry = json_object();
        json_object_set_new(entry, "output_module", json_string(moduleReference(cable->outputModule->id).c_str()));
        json_object_set_new(entry, "output_id", json_integer(cable->outputId));
        json_object_set_new(entry, "input_module", json_string(moduleReference(cable->inputModule->id).c_str()));
        json_object_set_new(entry, "input_id", json_integer(cable->inputId));
        json_array_append_new(cables, entry);
    }
    json_object_set_new(patch, "cables", cables);
    return patch;
}

std::string fingerprint(json_t* patch) {
    const std::string bytes = dump(patch);
    std::uint64_t hash = 1469598103934665603ULL;
    for (unsigned char byte : bytes) {
        hash ^= byte;
        hash *= 1099511628211ULL;
    }
    std::ostringstream stream;
    stream << std::hex << std::setfill('0') << std::setw(16) << hash;
    return stream.str();
}

json_t* snapshotJson(app::RackWidget* rack) {
    Json patch = owned(patchJson(rack));
    json_t* snapshot = json_object();
    const std::string revision = fingerprint(patch.get());
    json_object_set_new(snapshot, "fingerprint", json_string(revision.c_str()));
    json_object_set_new(snapshot, "catalog", catalogJson(rack));
    json_object_set(snapshot, "patch", patch.get());
    return snapshot;
}

struct ResolvedModule {
    app::ModuleWidget* widget;
    plugin::Model* model;
    ResolvedModule(app::ModuleWidget* widget = nullptr, plugin::Model* model = nullptr)
        : widget(widget), model(model) {}
};

ResolvedModule resolve(const std::map<std::string, ResolvedModule>& modules, const std::string& reference) {
    auto found = modules.find(reference);
    if (found == modules.end()) throw std::runtime_error("unknown module reference " + reference);
    return found->second;
}

engine::Module* inspectionModule(const ResolvedModule& resolved, std::vector<std::unique_ptr<engine::Module>>& temporary) {
    if (resolved.widget && resolved.widget->module) return resolved.widget->module;
    engine::Module* module = resolved.model ? resolved.model->createModule() : nullptr;
    if (!module) throw std::runtime_error("module cannot be inspected");
    temporary.emplace_back(module);
    return module;
}

void validateCommands(json_t* commands, app::RackWidget* rack) {
    if (!json_is_array(commands) || json_array_size(commands) == 0 || json_array_size(commands) > 128)
        throw std::runtime_error("commands must contain 1 to 128 items");
    std::map<std::string, ResolvedModule> modules;
    for (app::ModuleWidget* widget : rack->getModules()) {
        if (widget && widget->module && widget->model)
            modules[moduleReference(widget->module->id)] = ResolvedModule(widget, widget->model);
    }
    std::set<std::pair<std::string, int>> occupiedInputs;
    for (app::CableWidget* widget : rack->getCompleteCables()) {
        engine::Cable* cable = widget ? widget->getCable() : nullptr;
        if (cable && cable->inputModule)
            occupiedInputs.insert({moduleReference(cable->inputModule->id), cable->inputId});
    }

    std::size_t index = 0;
    json_t* command = nullptr;
    json_array_foreach(commands, index, command) {
        if (!json_is_object(command)) throw std::runtime_error("command must be an object");
        if (requiredString(command, "type") != "add_module") continue;
        const std::string reference = requiredString(command, "reference");
        if (reference.compare(0, 3, "ai:") != 0 || reference.size() > 80)
            throw std::runtime_error("new module references must begin ai:");
        if (modules.count(reference)) throw std::runtime_error("duplicate module reference " + reference);
        const std::string pluginSlug = requiredString(command, "plugin");
        const std::string modelSlug = requiredString(command, "model");
        plugin::Model* model = plugin::getModel(pluginSlug, modelSlug);
        if (!model || model->hidden) throw std::runtime_error("model is not installed: " + pluginSlug + "/" + modelSlug);
        if (pluginSlug == "ControlRoomBridge")
            throw std::runtime_error("the singleton Control Room bridge cannot be added by Copilot");
        const float x = requiredFloat(command, "x");
        const float y = requiredFloat(command, "y");
        if (x < 0.f || y < 0.f || x > 500.f || y > 20.f) throw std::runtime_error("module position is out of range");
        modules[reference] = ResolvedModule(nullptr, model);
    }

    std::vector<std::unique_ptr<engine::Module>> temporary;
    json_array_foreach(commands, index, command) {
        const std::string type = requiredString(command, "type");
        if (type == "add_module") continue;
        if (type == "set_param") {
            const std::string reference = requiredString(command, "module");
            const int paramId = requiredInt(command, "param_id");
            const float value = requiredFloat(command, "value");
            engine::Module* module = inspectionModule(resolve(modules, reference), temporary);
            if (paramId < 0 || static_cast<std::size_t>(paramId) >= module->paramQuantities.size() ||
                !module->paramQuantities[paramId])
                throw std::runtime_error("unknown parameter on " + reference);
            engine::ParamQuantity* quantity = module->paramQuantities[paramId];
            if (value < quantity->minValue || value > quantity->maxValue)
                throw std::runtime_error("parameter value is outside its valid range on " + reference);
        } else if (type == "connect") {
            const std::string outputReference = requiredString(command, "output_module");
            const std::string inputReference = requiredString(command, "input_module");
            const int outputId = requiredInt(command, "output_id");
            const int inputId = requiredInt(command, "input_id");
            engine::Module* output = inspectionModule(resolve(modules, outputReference), temporary);
            engine::Module* input = inspectionModule(resolve(modules, inputReference), temporary);
            if (outputId < 0 || static_cast<std::size_t>(outputId) >= output->outputs.size())
                throw std::runtime_error("unknown output on " + outputReference);
            if (inputId < 0 || static_cast<std::size_t>(inputId) >= input->inputs.size())
                throw std::runtime_error("unknown input on " + inputReference);
            if (!occupiedInputs.insert({inputReference, inputId}).second)
                throw std::runtime_error("input is already connected on " + inputReference);
        } else {
            throw std::runtime_error("unsupported command type " + type);
        }
    }
}

std::string applyCommands(json_t* root, app::RackWidget* rack, int& lastHistoryIndex) {
    Json currentPatch = owned(patchJson(rack));
    const std::string currentFingerprint = fingerprint(currentPatch.get());
    if (requiredString(root, "base_fingerprint") != currentFingerprint)
        throw std::runtime_error("Rack patch changed after the proposal was created");
    json_t* commands = json_object_get(root, "commands");
    validateCommands(commands, rack);

    std::map<std::string, ResolvedModule> modules;
    for (app::ModuleWidget* widget : rack->getModules()) {
        if (widget && widget->module && widget->model)
            modules[moduleReference(widget->module->id)] = ResolvedModule(widget, widget->model);
    }
    std::unique_ptr<history::ComplexAction> action(new history::ComplexAction());
    action->name = "apply Control Room Copilot patch";
    std::size_t index = 0;
    json_t* command = nullptr;

    struct StagedModule {
        std::string reference;
        plugin::Model* model;
        std::unique_ptr<app::ModuleWidget> widget;
        float x;
        float y;
    };
    std::vector<StagedModule> staged;
    json_array_foreach(commands, index, command) {
        if (requiredString(command, "type") != "add_module") continue;
        const std::string reference = requiredString(command, "reference");
        plugin::Model* model = plugin::getModel(requiredString(command, "plugin"), requiredString(command, "model"));
        engine::Module* module = model->createModule();
        if (!module) throw std::runtime_error("model could not create an engine module");
        app::ModuleWidget* widget = model->createModuleWidget(module);
        if (!widget) {
            delete module;
            throw std::runtime_error("model could not create a module widget");
        }
        StagedModule item;
        item.reference = reference;
        item.model = model;
        item.widget.reset(widget);
        item.x = requiredFloat(command, "x");
        item.y = requiredFloat(command, "y");
        staged.push_back(std::move(item));
    }
    try {
        for (StagedModule& item : staged) {
            app::ModuleWidget* widget = item.widget.release();
            rack->addModule(widget);
            rack->setModulePosNearest(widget, Vec(item.x * RACK_GRID_WIDTH, item.y * RACK_GRID_HEIGHT));
            history::ModuleAdd* added = new history::ModuleAdd();
            added->setModule(widget);
            action->push(added);
            modules[item.reference] = ResolvedModule(widget, item.model);
        }
        json_array_foreach(commands, index, command) {
            if (requiredString(command, "type") != "set_param") continue;
            ResolvedModule resolved = resolve(modules, requiredString(command, "module"));
            const int paramId = requiredInt(command, "param_id");
            const float value = requiredFloat(command, "value");
            history::ParamChange* changed = new history::ParamChange();
            changed->moduleId = resolved.widget->module->id;
            changed->paramId = paramId;
            changed->oldValue = APP->engine->getParamValue(resolved.widget->module, paramId);
            changed->newValue = value;
            APP->engine->setParamValue(resolved.widget->module, paramId, value);
            action->push(changed);
        }
        json_array_foreach(commands, index, command) {
            if (requiredString(command, "type") != "connect") continue;
            ResolvedModule output = resolve(modules, requiredString(command, "output_module"));
            ResolvedModule input = resolve(modules, requiredString(command, "input_module"));
            std::unique_ptr<engine::Cable> cable(new engine::Cable());
            cable->outputModule = output.widget->module;
            cable->outputId = requiredInt(command, "output_id");
            cable->inputModule = input.widget->module;
            cable->inputId = requiredInt(command, "input_id");
            APP->engine->addCable(cable.get());
            app::CableWidget* cableWidget = new app::CableWidget();
            cableWidget->color = rack->getNextCableColor();
            cableWidget->setCable(cable.release());
            rack->addCable(cableWidget);
            history::CableAdd* added = new history::CableAdd();
            added->setCable(cableWidget);
            action->push(added);
        }
    } catch (...) {
        action->undo();
        throw;
    }
    APP->history->push(action.release());
    lastHistoryIndex = APP->history->actionIndex;

    Json updatedPatch = owned(patchJson(rack));
    const std::string updatedFingerprint = fingerprint(updatedPatch.get());
    Json response = owned(json_object());
    json_object_set_new(response.get(), "message", json_string("Rack patch applied; native Undo is available"));
    json_object_set_new(response.get(), "fingerprint", json_string(updatedFingerprint.c_str()));
    return ok(response.get());
}

std::string undoLast(app::RackWidget* rack, int& lastHistoryIndex) {
    if (lastHistoryIndex < 0 || APP->history->actionIndex != lastHistoryIndex)
        throw std::runtime_error("the last Rack action is not a Copilot patch; refusing to undo user work");
    APP->history->undo();
    lastHistoryIndex = -1;
    Json patch = owned(patchJson(rack));
    const std::string updatedFingerprint = fingerprint(patch.get());
    Json response = owned(json_object());
    json_object_set_new(response.get(), "message", json_string("Last Copilot patch undone"));
    json_object_set_new(response.get(), "fingerprint", json_string(updatedFingerprint.c_str()));
    return ok(response.get());
}

}  // namespace

std::string handleRackAgentRequest(const std::string& payload, int& lastCopilotHistoryIndex) {
    try {
        json_error_t error{};
        Json root = owned(json_loadb(payload.data(), payload.size(), JSON_REJECT_DUPLICATES, &error));
        if (!root || !json_is_object(root.get()))
            return fail(std::string("invalid JSON request: ") + error.text);
        if (!APP || !APP->scene || !APP->scene->rack || !APP->engine || !APP->history)
            return fail("Rack UI is not ready");
        app::RackWidget* rack = APP->scene->rack;
        const std::string operation = requiredString(root.get(), "op");
        if (operation == "snapshot") {
            Json response = owned(json_object());
            json_object_set_new(response.get(), "snapshot", snapshotJson(rack));
            return ok(response.get());
        }
        if (operation == "apply") return applyCommands(root.get(), rack, lastCopilotHistoryIndex);
        if (operation == "undo") return undoLast(rack, lastCopilotHistoryIndex);
        return fail("unsupported Rack agent operation " + operation);
    } catch (const std::exception& error) {
        return fail(error.what());
    } catch (...) {
        return fail("unknown Rack agent failure");
    }
}

}  // namespace controlroom
