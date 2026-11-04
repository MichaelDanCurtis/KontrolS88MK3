import Foundation

struct CanvasPosition: Codable, Equatable, Sendable {
  var x: Double
  var y: Double
}

struct PatchDocument: Codable, Equatable, Sendable {
  static let currentSchemaVersion: UInt16 = 1
  var schemaVersion: UInt16 = Self.currentSchemaVersion
  var name: String
  var graph: ControlGraph

  func validateSchema() throws {
    guard schemaVersion == Self.currentSchemaVersion else {
      throw CocoaError(
        .fileReadCorruptFile,
        userInfo: [
          NSLocalizedDescriptionKey:
            "Unsupported Control Room patch schema \(schemaVersion); expected \(Self.currentSchemaVersion)."
        ])
    }
  }
}

struct PatchDocumentStore: Sendable {
  func load(from url: URL) throws -> PatchDocument {
    let document = try JSONDecoder().decode(PatchDocument.self, from: Data(contentsOf: url))
    try document.validateSchema()
    return document
  }

  func save(_ document: PatchDocument, to url: URL) throws {
    try document.validateSchema()
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    try encoder.encode(document).write(to: url, options: [.atomic])
  }
}

struct RecoveredDraft: Codable, Equatable, Sendable {
  var endpoint: String
  var baseRevision: UInt64
  var nodes: [GraphNode]
}

struct CanvasLayoutStore {
  private static let storageKey = "control-room.canvas-layouts.v1"
  private let defaults: UserDefaults

  init(defaults: UserDefaults = .standard) {
    self.defaults = defaults
  }

  func load(endpoint: String) -> [String: CanvasPosition] {
    guard let data = defaults.data(forKey: Self.storageKey),
      let layouts = try? JSONDecoder().decode(
        [String: [String: CanvasPosition]].self, from: data)
    else { return [:] }
    return layouts[endpoint] ?? [:]
  }

  func save(_ positions: [String: CanvasPosition], endpoint: String) {
    var layouts: [String: [String: CanvasPosition]] = [:]
    if let data = defaults.data(forKey: Self.storageKey),
      let decoded = try? JSONDecoder().decode(
        [String: [String: CanvasPosition]].self, from: data)
    {
      layouts = decoded
    }
    layouts[endpoint] = positions
    if let data = try? JSONEncoder().encode(layouts) {
      defaults.set(data, forKey: Self.storageKey)
    }
  }
}

struct GraphSnapshot: Codable, Equatable, Sendable {
  var protocolVersion: UInt16
  var revision: UInt64
  var graph: ControlGraph
  var meters: [NodeMeter]
  var limiter: LimiterTelemetry
  var runtimeTick: UInt64
  var runtimeRunning: Bool
  var persistenceWarning: String?
  var io: [IOStatus]

  enum CodingKeys: String, CodingKey {
    case revision, graph, meters, limiter, io
    case protocolVersion = "protocol_version"
    case runtimeTick = "runtime_tick"
    case runtimeRunning = "runtime_running"
    case persistenceWarning = "persistence_warning"
  }
}

enum IOConnectionState: String, Codable, Equatable, Sendable {
  case connected
  case disconnected
  case dryRun = "dry_run"
  case disabled
}

struct IOStatus: Codable, Equatable, Identifiable, Sendable {
  var id: String
  var label: String
  var transport: String
  var endpoint: String
  var state: IOConnectionState
  var connectAttempts: UInt64
  var reconnects: UInt64
  var received: UInt64
  var sent: UInt64
  var errors: UInt64
  var dropped: UInt64

  enum CodingKeys: String, CodingKey {
    case id, label, transport, endpoint, state, reconnects, received, sent, errors, dropped
    case connectAttempts = "connect_attempts"
  }
}

struct ControlCatalog: Equatable, Sendable {
  var modules: [ModuleCatalogEntry] = []
  var devices: [DeviceCatalogEntry] = []
  var externalInputs: [BindingCatalogEntry] = []
  var externalOutputs: [BindingCatalogEntry] = []

}

extension ControlCatalog: Codable {
  enum CodingKeys: String, CodingKey {
    case modules, devices
    case externalInputs = "external_inputs"
    case externalOutputs = "external_outputs"
  }

  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    modules = try container.decodeIfPresent([ModuleCatalogEntry].self, forKey: .modules) ?? []
    devices = try container.decodeIfPresent([DeviceCatalogEntry].self, forKey: .devices) ?? []
    externalInputs =
      try container.decodeIfPresent([BindingCatalogEntry].self, forKey: .externalInputs) ?? []
    externalOutputs =
      try container.decodeIfPresent([BindingCatalogEntry].self, forKey: .externalOutputs) ?? []
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    try container.encode(modules, forKey: .modules)
    try container.encode(devices, forKey: .devices)
    try container.encode(externalInputs, forKey: .externalInputs)
    try container.encode(externalOutputs, forKey: .externalOutputs)
  }
}

enum ModulePortDirection: String, Codable, Equatable, Sendable { case input, output }
enum ModuleSignalType: String, Codable, Equatable, Sendable { case control, gate }

struct ModulePort: Codable, Equatable, Identifiable, Sendable {
  var id: String
  var name: String
  var direction: ModulePortDirection
  var signal: ModuleSignalType
  var required: Bool
}

struct ModuleCatalogEntry: Codable, Equatable, Identifiable, Sendable {
  var id: String
  var name: String
  var category: String
  var summary: String
  var defaultSpec: NodeSpec
  var ports: [ModulePort]

  enum CodingKeys: String, CodingKey {
    case id, name, category, summary, ports
    case defaultSpec = "default_spec"
  }
}

struct DeviceCatalogEntry: Codable, Equatable, Identifiable, Sendable {
  var id: String
  var displayName: String
  var pack: String
  var port: String
  var channel: UInt8
  var parameters: [ParameterCatalogEntry]

  enum CodingKeys: String, CodingKey {
    case id, pack, port, channel, parameters
    case displayName = "display_name"
  }
}

struct ParameterCatalogEntry: Codable, Equatable, Identifiable, Sendable {
  var id: String
  var min: Double
  var max: Double
  var encoding: String
}

struct BindingCatalogEntry: Codable, Equatable, Identifiable, Sendable {
  var id: String
  var transport: String
  var min: Double
  var max: Double
}

func validateDraft(_ nodes: [GraphNode], catalog: ControlCatalog) -> [String] {
  guard !nodes.isEmpty else { return ["A patch must contain at least one module."] }
  var errors: [String] = []
  var indices: [String: Int] = [:]
  for (index, node) in nodes.enumerated() {
    if node.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
      errors.append("A module has an empty ID.")
    } else if indices.updateValue(index, forKey: node.id) != nil {
      errors.append("Module ID ‘\(node.id)’ is duplicated.")
    }
  }
  if !errors.isEmpty { return errors }

  for node in nodes {
    if let input = node.input, indices[input] == nil {
      errors.append("\(node.id) references missing input ‘\(input)’.")
    }
    switch node.spec {
    case .lfo(_, let frequency):
      if !frequency.isFinite || frequency <= 0 {
        errors.append("\(node.id) requires a positive finite frequency.")
      }
    case .sampleHold(let rate, _):
      if !rate.isFinite || rate <= 0 {
        errors.append("\(node.id) requires a positive finite sample rate.")
      }
    case .stepSequence(let rate, let values):
      if !rate.isFinite || rate <= 0 {
        errors.append("\(node.id) requires a positive finite step rate.")
      }
      if values.isEmpty { errors.append("\(node.id) requires at least one step.") }
      if values.count > 64 { errors.append("\(node.id) supports at most 64 steps.") }
      if values.contains(where: { !$0.isFinite }) {
        errors.append("\(node.id) contains a non-finite step value.")
      }
    case .slew(let rise, let fall):
      if !rise.isFinite || rise <= 0 || !fall.isFinite || fall <= 0 {
        errors.append("\(node.id) requires positive finite rise and fall rates.")
      }
    case .clamp(let minimum, let maximum):
      if !minimum.isFinite || !maximum.isFinite || minimum >= maximum {
        errors.append("\(node.id) requires a clamp minimum below its maximum.")
      }
    case .curve(let exponent):
      if !exponent.isFinite || exponent <= 0 {
        errors.append("\(node.id) requires a positive finite curve exponent.")
      }
    case .quantize(let steps):
      if steps < 2 { errors.append("\(node.id) requires at least two quantize steps.") }
    case .math(let operation, let operand):
      if !operand.isFinite { errors.append("\(node.id) requires a finite math operand.") }
      if operation == "divide" && operand == 0 {
        errors.append("\(node.id) cannot divide by zero.")
      }
    case .envelope(let gate, let attack, let decay, let sustain, let release):
      if indices[gate] == nil {
        errors.append("\(node.id) references missing envelope gate ‘\(gate)’.")
      }
      if !attack.isFinite || attack < 0 || !decay.isFinite || decay < 0
        || !sustain.isFinite || !(0...1).contains(sustain) || !release.isFinite || release < 0
      {
        errors.append("\(node.id) has invalid ADSR envelope values.")
      }
    case .macro(let binding, let defaultValue):
      if binding.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        errors.append("\(node.id) requires a macro binding.")
      }
      if !defaultValue.isFinite { errors.append("\(node.id) has a non-finite macro default.") }
      if !catalog.externalInputs.isEmpty,
        !catalog.externalInputs.contains(where: { $0.id == binding })
      {
        errors.append("\(node.id) references unavailable input binding ‘\(binding)’.")
      }
    case .scale(let minimum, let maximum):
      if !minimum.isFinite || !maximum.isFinite || minimum >= maximum {
        errors.append("\(node.id) requires a minimum below its maximum.")
      }
    case .sink(let device, let parameter):
      if !catalog.devices.isEmpty {
        guard let entry = catalog.devices.first(where: { $0.id == device }) else {
          errors.append("\(node.id) references unavailable device ‘\(device)’.")
          continue
        }
        if !entry.parameters.contains(where: { $0.id == parameter }) {
          errors.append("\(node.id) references unavailable parameter ‘\(parameter)’.")
        }
      }
    case .externalSource(let binding, let defaultValue):
      if binding.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        errors.append("\(node.id) requires an input binding.")
      }
      if !defaultValue.isFinite { errors.append("\(node.id) has a non-finite default value.") }
      if !catalog.externalInputs.isEmpty,
        !catalog.externalInputs.contains(where: { $0.id == binding })
      {
        errors.append("\(node.id) references unavailable input binding ‘\(binding)’.")
      }
    case .externalSink(let binding):
      if binding.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        errors.append("\(node.id) requires an output binding.")
      }
      if !catalog.externalOutputs.isEmpty,
        !catalog.externalOutputs.contains(where: { $0.id == binding })
      {
        errors.append("\(node.id) references unavailable output binding ‘\(binding)’.")
      }
    case .takeover(let control, let epsilon):
      if indices[control] == nil {
        errors.append("\(node.id) references missing pickup control ‘\(control)’.")
      }
      if !epsilon.isFinite || !(0...1).contains(epsilon) {
        errors.append("\(node.id) pickup tolerance must be between 0 and 1.")
      }
    }
  }

  var visit = Array(repeating: 0, count: nodes.count)
  func hasCycle(_ index: Int) -> Bool {
    if visit[index] == 1 { return true }
    if visit[index] == 2 { return false }
    visit[index] = 1
    let node = nodes[index]
    var dependencies = node.input.map { [$0] } ?? []
    if case .takeover(let control, _) = node.spec { dependencies.append(control) }
    if case .envelope(let gate, _, _, _, _) = node.spec { dependencies.append(gate) }
    for dependency in dependencies {
      if let next = indices[dependency], hasCycle(next) { return true }
    }
    visit[index] = 2
    return false
  }
  if nodes.indices.contains(where: hasCycle) {
    errors.append("The patch contains a signal or pickup-control cycle.")
  }
  return errors
}

struct ControlGraph: Codable, Equatable, Sendable {
  var nodes: [GraphNode]
}

struct GraphNode: Codable, Equatable, Identifiable, Sendable {
  var id: String
  var spec: NodeSpec
  var input: String?
}

struct NodeMeter: Codable, Equatable, Sendable {
  var id: String
  var value: Double
}

struct LimiterTelemetry: Codable, Equatable, Sendable {
  var sentUpdates: UInt64
  var sentMessages: UInt64
  var deduped: UInt64
  var parameterRateLimited: UInt64
  var busRateLimited: UInt64
  var busRateLimitedMessages: UInt64
  var unknownSlot: UInt64

  enum CodingKeys: String, CodingKey {
    case sentUpdates = "sent_updates"
    case sentMessages = "sent_messages"
    case deduped
    case parameterRateLimited = "parameter_rate_limited"
    case busRateLimited = "bus_rate_limited"
    case busRateLimitedMessages = "bus_rate_limited_messages"
    case unknownSlot = "unknown_slot"
  }
}

enum NodeSpec: Equatable, Sendable {
  case lfo(shape: String, frequency: Double)
  case sampleHold(rate: Double, seed: UInt64)
  case stepSequence(rate: Double, values: [Double])
  case slew(rise: Double, fall: Double)
  case clamp(min: Double, max: Double)
  case curve(exponent: Double)
  case quantize(steps: UInt16)
  case math(operation: String, operand: Double)
  case envelope(gate: String, attack: Double, decay: Double, sustain: Double, release: Double)
  case macro(binding: String, defaultValue: Double)
  case scale(min: Double, max: Double)
  case sink(device: String, parameter: String)
  case externalSource(binding: String, defaultValue: Double)
  case externalSink(binding: String)
  case takeover(control: String, epsilon: Double)

  var kind: NodeKind {
    switch self {
    case .lfo: .lfo
    case .sampleHold: .sampleHold
    case .stepSequence: .stepSequence
    case .slew: .slew
    case .clamp: .clamp
    case .curve: .curve
    case .quantize: .quantize
    case .math: .math
    case .envelope: .envelope
    case .macro: .macro
    case .scale: .scale
    case .sink: .sink
    case .externalSource: .externalSource
    case .externalSink: .externalSink
    case .takeover: .takeover
    }
  }

  var subtitle: String {
    switch self {
    case .lfo(let shape, let frequency):
      "\(shape.capitalized) · \(frequency.formatted(.number.precision(.fractionLength(2)))) Hz"
    case .sampleHold(let rate, let seed):
      "\(rate.formatted(.number.precision(.fractionLength(2)))) Hz · seed \(seed)"
    case .stepSequence(let rate, let values):
      "\(values.count) steps · \(rate.formatted(.number.precision(.fractionLength(2)))) Hz"
    case .slew(let rise, let fall): "↑ \(rise.formatted()) · ↓ \(fall.formatted()) /s"
    case .clamp(let min, let max): "\(min.formatted())…\(max.formatted())"
    case .curve(let exponent): "Exponent \(exponent.formatted())"
    case .quantize(let steps): "\(steps) levels"
    case .math(let operation, let operand): "\(operation.capitalized) \(operand.formatted())"
    case .envelope(_, let attack, let decay, let sustain, let release):
      "A \(attack.formatted()) · D \(decay.formatted()) · S \(sustain.formatted()) · R \(release.formatted())"
    case .macro(let binding, _): binding
    case .scale(let min, let max): "\(min.formatted()) → \(max.formatted())"
    case .sink(let device, let parameter): "\(device) · \(parameter)"
    case .externalSource(let binding, _): binding
    case .externalSink(let binding): binding
    case .takeover(let control, _): "Pickup from \(control.isEmpty ? "unassigned" : control)"
    }
  }

  var patchPorts: [ModulePort] {
    let input: (String, String, ModuleSignalType) -> ModulePort = { id, name, signal in
      ModulePort(id: id, name: name, direction: .input, signal: signal, required: true)
    }
    let output = ModulePort(
      id: "out", name: "OUT", direction: .output, signal: .control, required: false)
    switch self {
    case .lfo, .sampleHold, .stepSequence, .macro, .externalSource:
      return [output]
    case .envelope:
      return [input("gate", "GATE", .gate), output]
    case .sink, .externalSink:
      return [input("signal", "IN", .control)]
    case .takeover:
      return [input("signal", "IN", .control), input("control", "PICKUP", .control), output]
    case .slew, .clamp, .curve, .quantize, .math, .scale:
      return [input("signal", "IN", .control), output]
    }
  }
}

enum NodeKind: String, CaseIterable, Identifiable, Sendable {
  case lfo = "LFO"
  case sampleHold = "Sample & Hold"
  case stepSequence = "Step Sequence"
  case slew = "Slew"
  case clamp = "Clamp"
  case curve = "Curve"
  case quantize = "Quantize"
  case math = "Math"
  case envelope = "Envelope"
  case macro = "Macro"
  case scale = "Scale"
  case sink = "Device Out"
  case externalSource = "Control In"
  case externalSink = "Signal Out"
  case takeover = "Pickup"

  var id: String { rawValue }

  var systemImage: String {
    switch self {
    case .lfo: "waveform.path"
    case .sampleHold: "dice"
    case .stepSequence: "chart.bar.xaxis"
    case .slew: "water.waves"
    case .clamp: "arrow.down.and.line.horizontal.and.arrow.up"
    case .curve: "point.topleft.down.to.point.bottomright.curvepath"
    case .quantize: "square.grid.3x3"
    case .math: "function"
    case .envelope: "waveform.path.ecg"
    case .macro: "slider.horizontal.3"
    case .scale: "arrow.up.right"
    case .sink: "cable.connector"
    case .externalSource: "dial.medium"
    case .externalSink: "dot.radiowaves.right"
    case .takeover: "hand.draw"
    }
  }

  var tintName: String {
    switch self {
    case .lfo: "cyan"
    case .sampleHold: "mint"
    case .stepSequence: "blue"
    case .slew: "violet"
    case .clamp: "coral"
    case .curve: "cyan"
    case .quantize: "amber"
    case .math: "mint"
    case .envelope: "coral"
    case .macro: "amber"
    case .scale: "violet"
    case .sink: "amber"
    case .externalSource: "mint"
    case .externalSink: "blue"
    case .takeover: "coral"
    }
  }

  func makeSpec() -> NodeSpec {
    switch self {
    case .lfo: .lfo(shape: "triangle", frequency: 0.3)
    case .sampleHold: .sampleHold(rate: 2, seed: 1)
    case .stepSequence: .stepSequence(rate: 4, values: [0, 0.35, 0.75, 1])
    case .slew: .slew(rise: 1, fall: 1)
    case .clamp: .clamp(min: 0, max: 1)
    case .curve: .curve(exponent: 2)
    case .quantize: .quantize(steps: 8)
    case .math: .math(operation: "multiply", operand: 1)
    case .envelope:
      .envelope(gate: "", attack: 0.05, decay: 0.2, sustain: 0.7, release: 0.4)
    case .macro: .macro(binding: "s88.knob.1", defaultValue: 0)
    case .scale: .scale(min: 0, max: 127)
    case .sink: .sink(device: "peak", parameter: "filter.cutoff")
    case .externalSource: .externalSource(binding: "s88.knob.1", defaultValue: 0)
    case .externalSink: .externalSink(binding: "osc.monitor")
    case .takeover: .takeover(control: "", epsilon: 0.01)
    }
  }

  var fallbackModule: ModuleCatalogEntry {
    let slug = rawValue.lowercased().replacingOccurrences(of: " ", with: "-")
      .replacingOccurrences(of: "&", with: "and")
    let spec = makeSpec()
    return ModuleCatalogEntry(
      id: slug, name: rawValue, category: "Core", summary: spec.subtitle,
      defaultSpec: spec, ports: spec.patchPorts)
  }
}

private struct LfoPayload: Codable {
  var shape: String
  var freqHz: Double
  enum CodingKeys: String, CodingKey {
    case shape
    case freqHz = "freq_hz"
  }
}
private struct SampleHoldPayload: Codable {
  var rateHz: Double
  var seed: UInt64
  enum CodingKeys: String, CodingKey {
    case rateHz = "rate_hz"
    case seed
  }
}
private struct StepSequencePayload: Codable {
  var rateHz: Double
  var values: [Double]
  enum CodingKeys: String, CodingKey {
    case rateHz = "rate_hz"
    case values
  }
}
private struct SlewPayload: Codable {
  var risePerSecond: Double
  var fallPerSecond: Double
  enum CodingKeys: String, CodingKey {
    case risePerSecond = "rise_per_second"
    case fallPerSecond = "fall_per_second"
  }
}
private struct ClampPayload: Codable {
  var min: Double
  var max: Double
}
private struct CurvePayload: Codable { var exponent: Double }
private struct QuantizePayload: Codable { var steps: UInt16 }
private struct MathPayload: Codable {
  var operation: String
  var operand: Double
}
private struct EnvelopePayload: Codable {
  var gate: String
  var attackSeconds: Double
  var decaySeconds: Double
  var sustain: Double
  var releaseSeconds: Double
  enum CodingKeys: String, CodingKey {
    case gate, sustain
    case attackSeconds = "attack_seconds"
    case decaySeconds = "decay_seconds"
    case releaseSeconds = "release_seconds"
  }
}
private struct MacroPayload: Codable {
  var binding: String
  var defaultValue: Double
  enum CodingKeys: String, CodingKey {
    case binding
    case defaultValue = "default"
  }
}
private struct ScalePayload: Codable {
  var min: Double
  var max: Double
}
private struct SinkPayload: Codable {
  var device: String
  var param: String
}
private struct ExternalSourcePayload: Codable {
  var binding: String
  var defaultValue: Double
  enum CodingKeys: String, CodingKey {
    case binding
    case defaultValue = "default"
  }
}
private struct ExternalSinkPayload: Codable { var binding: String }
private struct TakeoverPayload: Codable {
  var control: String
  var pickupEpsilon: Double
  enum CodingKeys: String, CodingKey {
    case control
    case pickupEpsilon = "pickup_epsilon"
  }
}

extension NodeSpec: Codable {
  private enum Keys: String, CodingKey {
    case lfo = "Lfo"
    case sampleHold = "SampleHold"
    case stepSequence = "StepSequence"
    case slew = "Slew"
    case clamp = "Clamp"
    case curve = "Curve"
    case quantize = "Quantize"
    case math = "Math"
    case envelope = "Envelope"
    case macro = "Macro"
    case scale = "Scale"
    case sink = "Sink"
    case externalSource = "ExternalSource"
    case externalSink = "ExternalSink"
    case takeover = "Takeover"
  }

  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: Keys.self)
    if container.contains(.lfo) {
      let value = try container.decode(LfoPayload.self, forKey: .lfo)
      self = .lfo(shape: value.shape, frequency: value.freqHz)
    } else if container.contains(.sampleHold) {
      let value = try container.decode(SampleHoldPayload.self, forKey: .sampleHold)
      self = .sampleHold(rate: value.rateHz, seed: value.seed)
    } else if container.contains(.stepSequence) {
      let value = try container.decode(StepSequencePayload.self, forKey: .stepSequence)
      self = .stepSequence(rate: value.rateHz, values: value.values)
    } else if container.contains(.slew) {
      let value = try container.decode(SlewPayload.self, forKey: .slew)
      self = .slew(rise: value.risePerSecond, fall: value.fallPerSecond)
    } else if container.contains(.clamp) {
      let value = try container.decode(ClampPayload.self, forKey: .clamp)
      self = .clamp(min: value.min, max: value.max)
    } else if container.contains(.curve) {
      let value = try container.decode(CurvePayload.self, forKey: .curve)
      self = .curve(exponent: value.exponent)
    } else if container.contains(.quantize) {
      let value = try container.decode(QuantizePayload.self, forKey: .quantize)
      self = .quantize(steps: value.steps)
    } else if container.contains(.math) {
      let value = try container.decode(MathPayload.self, forKey: .math)
      self = .math(operation: value.operation, operand: value.operand)
    } else if container.contains(.envelope) {
      let value = try container.decode(EnvelopePayload.self, forKey: .envelope)
      self = .envelope(
        gate: value.gate, attack: value.attackSeconds, decay: value.decaySeconds,
        sustain: value.sustain, release: value.releaseSeconds)
    } else if container.contains(.macro) {
      let value = try container.decode(MacroPayload.self, forKey: .macro)
      self = .macro(binding: value.binding, defaultValue: value.defaultValue)
    } else if container.contains(.scale) {
      let value = try container.decode(ScalePayload.self, forKey: .scale)
      self = .scale(min: value.min, max: value.max)
    } else if container.contains(.sink) {
      let value = try container.decode(SinkPayload.self, forKey: .sink)
      self = .sink(device: value.device, parameter: value.param)
    } else if container.contains(.externalSource) {
      let value = try container.decode(ExternalSourcePayload.self, forKey: .externalSource)
      self = .externalSource(binding: value.binding, defaultValue: value.defaultValue)
    } else if container.contains(.externalSink) {
      let value = try container.decode(ExternalSinkPayload.self, forKey: .externalSink)
      self = .externalSink(binding: value.binding)
    } else if container.contains(.takeover) {
      let value = try container.decode(TakeoverPayload.self, forKey: .takeover)
      self = .takeover(control: value.control, epsilon: value.pickupEpsilon)
    } else {
      throw DecodingError.dataCorrupted(
        .init(codingPath: decoder.codingPath, debugDescription: "Unknown node specification"))
    }
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: Keys.self)
    switch self {
    case .lfo(let shape, let frequency):
      try container.encode(LfoPayload(shape: shape, freqHz: frequency), forKey: .lfo)
    case .sampleHold(let rate, let seed):
      try container.encode(SampleHoldPayload(rateHz: rate, seed: seed), forKey: .sampleHold)
    case .stepSequence(let rate, let values):
      try container.encode(
        StepSequencePayload(rateHz: rate, values: values), forKey: .stepSequence)
    case .slew(let rise, let fall):
      try container.encode(
        SlewPayload(risePerSecond: rise, fallPerSecond: fall), forKey: .slew)
    case .clamp(let min, let max):
      try container.encode(ClampPayload(min: min, max: max), forKey: .clamp)
    case .curve(let exponent):
      try container.encode(CurvePayload(exponent: exponent), forKey: .curve)
    case .quantize(let steps):
      try container.encode(QuantizePayload(steps: steps), forKey: .quantize)
    case .math(let operation, let operand):
      try container.encode(MathPayload(operation: operation, operand: operand), forKey: .math)
    case .envelope(let gate, let attack, let decay, let sustain, let release):
      try container.encode(
        EnvelopePayload(
          gate: gate, attackSeconds: attack, decaySeconds: decay, sustain: sustain,
          releaseSeconds: release), forKey: .envelope)
    case .macro(let binding, let defaultValue):
      try container.encode(
        MacroPayload(binding: binding, defaultValue: defaultValue), forKey: .macro)
    case .scale(let min, let max):
      try container.encode(ScalePayload(min: min, max: max), forKey: .scale)
    case .sink(let device, let parameter):
      try container.encode(SinkPayload(device: device, param: parameter), forKey: .sink)
    case .externalSource(let binding, let defaultValue):
      try container.encode(
        ExternalSourcePayload(binding: binding, defaultValue: defaultValue), forKey: .externalSource
      )
    case .externalSink(let binding):
      try container.encode(ExternalSinkPayload(binding: binding), forKey: .externalSink)
    case .takeover(let control, let epsilon):
      try container.encode(
        TakeoverPayload(control: control, pickupEpsilon: epsilon), forKey: .takeover)
    }
  }
}

enum GraphCommand: Encodable, Sendable {
  case addNode(id: String, spec: NodeSpec)
  case removeNode(id: String)
  case connect(source: String, target: String)

  private enum Keys: String, CodingKey {
    case addNode = "AddNode"
    case removeNode = "RemoveNode"
    case connect = "Connect"
  }
  private struct Add: Encodable {
    var id: String
    var spec: NodeSpec
  }
  private struct Remove: Encodable { var id: String }
  private struct Connect: Encodable {
    var source: String
    var target: String
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: Keys.self)
    switch self {
    case .addNode(let id, let spec): try container.encode(Add(id: id, spec: spec), forKey: .addNode)
    case .removeNode(let id): try container.encode(Remove(id: id), forKey: .removeNode)
    case .connect(let source, let target):
      try container.encode(Connect(source: source, target: target), forKey: .connect)
    }
  }
}

struct CommandBatch: Encodable, Sendable {
  var baseRevision: UInt64
  var commands: [GraphCommand]
  enum CodingKeys: String, CodingKey {
    case baseRevision = "base_revision"
    case commands
  }
}

struct CommandResponse: Decodable, Sendable {
  var revision: UInt64
  var persisted: Bool
  var warning: String?
}

struct ServerEventEnvelope: Decodable, Sendable {
  var type: String
  var payload: GraphSnapshot?
}

struct APIErrorPayload: Decodable, Sendable { var error: String }

struct RackPort: Codable, Equatable, Identifiable, Sendable {
  var id: Int
  var name: String
}

struct RackParameter: Codable, Equatable, Identifiable, Sendable {
  var id: Int
  var name: String
  var min: Double
  var max: Double
  var `default`: Double
}

struct RackModel: Codable, Equatable, Identifiable, Sendable {
  var plugin: String
  var model: String
  var name: String
  var description: String
  var tags: [String]
  var params: [RackParameter]
  var inputs: [RackPort]
  var outputs: [RackPort]
  var id: String { "\(plugin)/\(model)" }
}

struct RackPatchModule: Codable, Equatable, Identifiable, Sendable {
  var reference: String
  var rackID: Int64
  var plugin: String
  var model: String
  var name: String
  var x: Double
  var y: Double
  var id: String { reference }

  enum CodingKeys: String, CodingKey {
    case reference, plugin, model, name, x, y
    case rackID = "rack_id"
  }
}

struct RackPatchSummary: Codable, Equatable, Sendable {
  var modules: [RackPatchModule]
}

struct RackSnapshot: Codable, Equatable, Sendable {
  var fingerprint: String
  var catalog: [RackModel]
  var patch: RackPatchSummary
}

struct RackCommandPreview: Codable, Equatable, Identifiable, Sendable {
  var type: String
  var reference: String?
  var plugin: String?
  var model: String?
  var x: Double?
  var y: Double?
  var module: String?
  var paramID: Int?
  var value: Double?
  var outputModule: String?
  var outputID: Int?
  var inputModule: String?
  var inputID: Int?
  var id = UUID()

  enum CodingKeys: String, CodingKey {
    case type, reference, plugin, model, x, y, module, value
    case paramID = "param_id"
    case outputModule = "output_module"
    case outputID = "output_id"
    case inputModule = "input_module"
    case inputID = "input_id"
  }

  var label: String {
    switch type {
    case "add_module": "Add \(plugin ?? "?") / \(model ?? "?") as \(reference ?? "?")"
    case "set_param":
      "Set parameter \(paramID ?? -1) on \(module ?? "?") to \(value?.formatted(.number.precision(.significantDigits(4))) ?? "?")"
    case "connect":
      "Patch \(outputModule ?? "?") [\(outputID ?? -1)] → \(inputModule ?? "?") [\(inputID ?? -1)]"
    default: type.replacingOccurrences(of: "_", with: " ")
    }
  }
}

struct RackLibrarySuggestion: Codable, Equatable, Sendable {
  var query: String
  var reason: String
}

struct RackProposal: Codable, Equatable, Identifiable, Sendable {
  var id: String
  var prompt: String
  var baseFingerprint: String
  var provider: String?
  var model: String
  var summary: String
  var warnings: [String]
  var commands: [RackCommandPreview]
  var librarySuggestions: [RackLibrarySuggestion]

  enum CodingKeys: String, CodingKey {
    case id, prompt, provider, model, summary, warnings, commands
    case baseFingerprint = "base_fingerprint"
    case librarySuggestions = "library_suggestions"
  }
}

struct RackMutationResponse: Codable, Equatable, Sendable {
  var message: String
  var fingerprint: String
}

struct RackProposePayload: Encodable, Sendable {
  var prompt: String
  var provider: String?
}
struct RackApplyPayload: Encodable, Sendable {
  var proposalID: String
  enum CodingKeys: String, CodingKey { case proposalID = "proposal_id" }
}
