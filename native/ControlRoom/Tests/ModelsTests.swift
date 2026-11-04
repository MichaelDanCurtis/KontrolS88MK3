import Foundation
import Testing

@testable import ControlRoomNative

@Test func rackProposalDecodesTypedPreviewCommands() throws {
  let json = #"""
    {
      "id":"rack-1","prompt":"add a voice","base_fingerprint":"abc","provider":"openai","model":"gpt-5.4-mini",
      "summary":"Add one oscillator","warnings":[],
      "commands":[
        {"type":"add_module","reference":"ai:vco","plugin":"Fundamental","model":"VCO","x":8,"y":0},
        {"type":"set_param","module":"ai:vco","param_id":0,"value":1.25}
      ],
      "library_suggestions":[{"query":"stereo reverb","reason":"No reverb is installed"}]
    }
    """#
  let proposal = try JSONDecoder().decode(RackProposal.self, from: Data(json.utf8))
  #expect(proposal.provider == "openai")
  #expect(proposal.commands.count == 2)
  #expect(proposal.commands[0].label.contains("Fundamental / VCO"))
  #expect(proposal.commands[1].paramID == 0)
  #expect(proposal.librarySuggestions.first?.query == "stereo reverb")
}

@Test func decodesAndReencodesEveryNodeKind() throws {
  let json =
    #"{"nodes":[{"id":"lfo","spec":{"Lfo":{"shape":"triangle","freq_hz":0.3}},"input":null},{"id":"random","spec":{"SampleHold":{"rate_hz":2.0,"seed":42}},"input":null},{"id":"steps","spec":{"StepSequence":{"rate_hz":4.0,"values":[0.0,0.5,1.0]}},"input":null},{"id":"slew","spec":{"Slew":{"rise_per_second":1.0,"fall_per_second":2.0}},"input":"lfo"},{"id":"clamp","spec":{"Clamp":{"min":0.0,"max":1.0}},"input":"slew"},{"id":"curve","spec":{"Curve":{"exponent":2.0}},"input":"clamp"},{"id":"quantize","spec":{"Quantize":{"steps":8}},"input":"curve"},{"id":"math","spec":{"Math":{"operation":"multiply","operand":2.0}},"input":"quantize"},{"id":"macro","spec":{"Macro":{"binding":"s88.knob.1","default":0.0}},"input":null},{"id":"envelope","spec":{"Envelope":{"gate":"macro","attack_seconds":0.1,"decay_seconds":0.2,"sustain":0.6,"release_seconds":0.3}},"input":null},{"id":"scale","spec":{"Scale":{"min":0.0,"max":127.0}},"input":"math"},{"id":"sink","spec":{"Sink":{"device":"peak","param":"filter.cutoff"}},"input":"scale"},{"id":"knob","spec":{"ExternalSource":{"binding":"s88.knob.1","default":0.0}},"input":null},{"id":"pickup","spec":{"Takeover":{"control":"knob","pickup_epsilon":0.01}},"input":"lfo"},{"id":"monitor","spec":{"ExternalSink":{"binding":"osc.monitor"}},"input":"pickup"}]}"#
  let graph = try JSONDecoder().decode(ControlGraph.self, from: Data(json.utf8))
  #expect(graph.nodes.count == 15)
  #expect(graph.nodes[1].spec == .sampleHold(rate: 2, seed: 42))
  #expect(graph.nodes[2].spec == .stepSequence(rate: 4, values: [0, 0.5, 1]))
  #expect(graph.nodes[3].spec == .slew(rise: 1, fall: 2))
  #expect(graph.nodes[7].spec == .math(operation: "multiply", operand: 2))
  #expect(graph.nodes[8].spec == .macro(binding: "s88.knob.1", defaultValue: 0))
  #expect(
    graph.nodes[9].spec
      == .envelope(gate: "macro", attack: 0.1, decay: 0.2, sustain: 0.6, release: 0.3))
  #expect(graph.nodes[13].spec == .takeover(control: "knob", epsilon: 0.01))
  let encoded = try JSONEncoder().encode(graph)
  #expect(try JSONDecoder().decode(ControlGraph.self, from: encoded) == graph)
}

@Test func replacementCommandsMatchRustExternalTagging() throws {
  let batch = CommandBatch(
    baseRevision: 7,
    commands: [
      .removeNode(id: "old"),
      .addNode(id: "new", spec: .scale(min: 0, max: 1)),
      .connect(source: "source", target: "new"),
    ])
  let object = try #require(
    JSONSerialization.jsonObject(with: JSONEncoder().encode(batch)) as? [String: Any])
  #expect(object["base_revision"] as? Int == 7)
  let commands = try #require(object["commands"] as? [[String: Any]])
  #expect(commands[0]["RemoveNode"] != nil)
  #expect(commands[1]["AddNode"] != nil)
  #expect(commands[2]["Connect"] != nil)
}

@Test func canvasLayoutsAreStoredPerEndpoint() throws {
  let suite = try #require(UserDefaults(suiteName: "ControlRoomNativeTests.CanvasLayout"))
  suite.removePersistentDomain(forName: "ControlRoomNativeTests.CanvasLayout")
  let store = CanvasLayoutStore(defaults: suite)
  store.save(["lfo": CanvasPosition(x: 12, y: 34)], endpoint: "http://studio-a")
  store.save(["sink": CanvasPosition(x: 56, y: 78)], endpoint: "http://studio-b")
  #expect(store.load(endpoint: "http://studio-a")["lfo"] == CanvasPosition(x: 12, y: 34))
  #expect(store.load(endpoint: "http://studio-b")["sink"] == CanvasPosition(x: 56, y: 78))
  #expect(store.load(endpoint: "http://unknown").isEmpty)
}

@Test @MainActor func remoteRevisionBlocksAStaleDraft() throws {
  let suite = try #require(UserDefaults(suiteName: "ControlRoomNativeTests.RevisionConflict"))
  suite.removePersistentDomain(forName: "ControlRoomNativeTests.RevisionConflict")
  let client = ControlRoomClient(preferences: suite)
  client.accept(snapshot(revision: 4))
  client.markDirty()
  client.accept(snapshot(revision: 5))
  #expect(client.dirty)
  #expect(client.hasRevisionConflict)
  #expect(client.notice?.contains("Revision conflict") == true)
  client.markDirty()
  #expect(client.hasRevisionConflict)

  client.discardChanges()
  #expect(!client.dirty)
  #expect(!client.hasRevisionConflict)
  #expect(client.revision == 5)
}

private func snapshot(revision: UInt64, nodes: [GraphNode]? = nil) -> GraphSnapshot {
  GraphSnapshot(
    protocolVersion: 1,
    revision: revision,
    graph: ControlGraph(
      nodes: nodes
        ?? [GraphNode(id: "lfo", spec: .lfo(shape: "triangle", frequency: 0.3), input: nil)]),
    meters: [],
    limiter: LimiterTelemetry(
      sentUpdates: 0,
      sentMessages: 0,
      deduped: 0,
      parameterRateLimited: 0,
      busRateLimited: 0,
      busRateLimitedMessages: 0,
      unknownSlot: 0),
    runtimeTick: 0,
    runtimeRunning: true,
    persistenceWarning: nil,
    io: [])
}

@Test func decodesCatalogAndUnifiedIOStatus() throws {
  let catalogJSON =
    #"{"devices":[{"id":"peak","display_name":"Novation Peak","pack":"novation-peak","port":"Peak","channel":1,"parameters":[{"id":"filter.cutoff","min":0.0,"max":127.0,"encoding":"cc7"}]}],"external_inputs":[],"external_outputs":[]}"#
  let catalog = try JSONDecoder().decode(ControlCatalog.self, from: Data(catalogJSON.utf8))
  #expect(catalog.devices[0].parameters[0].id == "filter.cutoff")

  let statusJSON =
    #"{"id":"peak","label":"peak","transport":"midi_out","endpoint":"Peak MIDI","state":"connected","connect_attempts":1,"reconnects":0,"received":0,"sent":12,"errors":0,"dropped":0}"#
  let status = try JSONDecoder().decode(IOStatus.self, from: Data(statusJSON.utf8))
  #expect(status.state == .connected)
  #expect(status.sent == 12)
}

@Test func moduleCatalogDecodesTypedPortsAndDefaults() throws {
  let json =
    #"{"modules":[{"id":"slow-triangle","name":"Slow Triangle","category":"Modulators","summary":"Broad motion","default_spec":{"Lfo":{"shape":"triangle","freq_hz":0.08}},"ports":[{"id":"out","name":"OUT","direction":"output","signal":"control","required":false}]}],"devices":[],"external_inputs":[],"external_outputs":[]}"#
  let catalog = try JSONDecoder().decode(ControlCatalog.self, from: Data(json.utf8))
  #expect(catalog.modules.count == 1)
  #expect(catalog.modules[0].defaultSpec == .lfo(shape: "triangle", frequency: 0.08))
  #expect(catalog.modules[0].ports[0].direction == .output)
  #expect(catalog.modules[0].ports[0].signal == .control)

  #expect(NodeKind.envelope.makeSpec().patchPorts.map(\.id) == ["gate", "out"])
  #expect(NodeKind.takeover.makeSpec().patchPorts.map(\.id) == ["signal", "control", "out"])
  #expect(NodeKind.sink.makeSpec().patchPorts.map(\.id) == ["signal"])
}

@Test @MainActor func directJackPatchingConnectsAuxiliaryPortsAndSupportsUndo() throws {
  let suite = try #require(UserDefaults(suiteName: "ControlRoomNativeTests.PatchPorts"))
  suite.removePersistentDomain(forName: "ControlRoomNativeTests.PatchPorts")
  let client = ControlRoomClient(preferences: suite)
  let nodes = [
    GraphNode(id: "mod", spec: .lfo(shape: "triangle", frequency: 1), input: nil),
    GraphNode(id: "knob", spec: .macro(binding: "s88.knob.1", defaultValue: 0), input: nil),
    GraphNode(
      id: "env",
      spec: .envelope(gate: "", attack: 0.1, decay: 0.2, sustain: 0.6, release: 0.3),
      input: nil),
    GraphNode(id: "pickup", spec: .takeover(control: "", epsilon: 0.01), input: nil),
  ]
  client.accept(snapshot(revision: 1, nodes: nodes))

  client.patch(source: "knob", target: "env", portID: "gate")
  #expect(
    client.workingNodes[2].spec
      == .envelope(gate: "knob", attack: 0.1, decay: 0.2, sustain: 0.6, release: 0.3))
  client.patch(source: "mod", target: "pickup", portID: "signal")
  client.patch(source: "knob", target: "pickup", portID: "control")
  #expect(client.workingNodes[3].input == "mod")
  #expect(client.workingNodes[3].spec == .takeover(control: "knob", epsilon: 0.01))

  client.disconnect(target: "pickup", portID: "control")
  #expect(client.workingNodes[3].spec == .takeover(control: "", epsilon: 0.01))
  client.undo()
  #expect(client.workingNodes[3].spec == .takeover(control: "knob", epsilon: 0.01))
}

@Test func draftValidationCatchesSemanticAndCycleErrors() {
  let catalog = ControlCatalog(
    devices: [
      DeviceCatalogEntry(
        id: "peak", displayName: "Peak", pack: "peak", port: "Peak", channel: 1,
        parameters: [
          ParameterCatalogEntry(id: "filter.cutoff", min: 0, max: 127, encoding: "cc7")
        ])
    ])
  let nodes = [
    GraphNode(id: "a", spec: .scale(min: 1, max: 0), input: "b"),
    GraphNode(id: "b", spec: .sink(device: "peak", parameter: "missing"), input: "a"),
  ]
  let errors = validateDraft(nodes, catalog: catalog)
  #expect(errors.contains(where: { $0.contains("minimum") }))
  #expect(errors.contains(where: { $0.contains("unavailable parameter") }))
  #expect(errors.contains(where: { $0.contains("cycle") }))
}

@Test func envelopeGateParticipatesInValidationAndCycles() {
  let nodes = [
    GraphNode(
      id: "env",
      spec: .envelope(gate: "scaled", attack: 0.1, decay: 0.2, sustain: 0.5, release: 0.3),
      input: nil),
    GraphNode(id: "scaled", spec: .scale(min: 0, max: 1), input: "env"),
  ]
  let errors = validateDraft(nodes, catalog: ControlCatalog())
  #expect(errors.contains(where: { $0.contains("cycle") }))
}

@Test @MainActor func renamingAndRemovingGateRepairsEnvelopeReferences() throws {
  let suite = try #require(UserDefaults(suiteName: "ControlRoomNativeTests.EnvelopeReferences"))
  suite.removePersistentDomain(forName: "ControlRoomNativeTests.EnvelopeReferences")
  let client = ControlRoomClient(preferences: suite)
  let nodes = [
    GraphNode(id: "gate", spec: .macro(binding: "control", defaultValue: 0), input: nil),
    GraphNode(
      id: "env",
      spec: .envelope(gate: "gate", attack: 0.1, decay: 0.2, sustain: 0.5, release: 0.3),
      input: nil),
  ]
  client.accept(snapshot(revision: 1, nodes: nodes))
  client.updateNode(
    GraphNode(id: "renamed", spec: .macro(binding: "control", defaultValue: 0), input: nil),
    replacing: "gate")
  #expect(
    client.workingNodes[1].spec
      == .envelope(gate: "renamed", attack: 0.1, decay: 0.2, sustain: 0.5, release: 0.3))
  client.remove("renamed")
  #expect(
    client.workingNodes[0].spec
      == .envelope(gate: "", attack: 0.1, decay: 0.2, sustain: 0.5, release: 0.3))
}

@Test @MainActor func localHistorySupportsUndoAndRedo() throws {
  let suite = try #require(UserDefaults(suiteName: "ControlRoomNativeTests.History"))
  suite.removePersistentDomain(forName: "ControlRoomNativeTests.History")
  let client = ControlRoomClient(preferences: suite)
  client.accept(snapshot(revision: 1))
  client.add(.scale)
  #expect(client.workingNodes.count == 2)
  #expect(client.canUndo)
  client.undo()
  #expect(client.workingNodes.count == 1)
  #expect(!client.dirty)
  #expect(client.canRedo)
  client.accept(snapshot(revision: 1))
  #expect(client.canRedo)
  client.redo()
  #expect(client.workingNodes.count == 2)
  #expect(client.dirty)
}

@Test func patchDocumentsRoundTripAtomicallyAndRejectFutureSchemas() throws {
  let url = FileManager.default.temporaryDirectory.appending(
    path: "control-room-\(UUID().uuidString).controlroompatch")
  defer { try? FileManager.default.removeItem(at: url) }
  let store = PatchDocumentStore()
  let document = PatchDocument(
    name: "Filter Motion",
    graph: ControlGraph(
      nodes: [GraphNode(id: "lfo", spec: .lfo(shape: "sine", frequency: 0.25), input: nil)]))
  try store.save(document, to: url)
  #expect(try store.load(from: url) == document)

  var future = document
  future.schemaVersion = 99
  #expect(throws: (any Error).self) { try store.save(future, to: url) }
}

@Test @MainActor func documentLoadingIsStagedAndAddedToRecents() throws {
  let suite = try #require(UserDefaults(suiteName: "ControlRoomNativeTests.Documents"))
  suite.removePersistentDomain(forName: "ControlRoomNativeTests.Documents")
  let url = FileManager.default.temporaryDirectory.appending(
    path: "control-room-\(UUID().uuidString).controlroompatch")
  defer { try? FileManager.default.removeItem(at: url) }
  let document = PatchDocument(
    name: "Loaded",
    graph: ControlGraph(
      nodes: [GraphNode(id: "loaded", spec: .lfo(shape: "triangle", frequency: 1), input: nil)]))
  try PatchDocumentStore().save(document, to: url)
  let client = ControlRoomClient(preferences: suite)
  client.accept(snapshot(revision: 2))
  try client.loadDocument(from: url)
  #expect(client.workingNodes[0].id == "loaded")
  #expect(client.dirty)
  #expect(client.recentDocuments.first == url)
}

@Test @MainActor func interruptedDraftRequiresExplicitRestore() throws {
  let suite = try #require(UserDefaults(suiteName: "ControlRoomNativeTests.Recovery"))
  suite.removePersistentDomain(forName: "ControlRoomNativeTests.Recovery")
  let first = ControlRoomClient(preferences: suite)
  first.accept(snapshot(revision: 3))
  first.add(.scale)

  let relaunched = ControlRoomClient(preferences: suite)
  relaunched.accept(snapshot(revision: 3))
  #expect(relaunched.recoveredDraftAvailable)
  #expect(relaunched.workingNodes.count == 1)
  relaunched.restoreRecoveredDraft()
  #expect(relaunched.workingNodes.count == 2)
  #expect(relaunched.dirty)
}
