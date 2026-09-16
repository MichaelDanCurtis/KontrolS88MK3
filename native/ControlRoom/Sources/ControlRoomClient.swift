import AppKit
import Combine
import Foundation
import UniformTypeIdentifiers

@MainActor
final class ControlRoomClient: ObservableObject {
  private static let endpointPreferenceKey = "control-room.endpoint.v1"
  private static let recentDocumentsPreferenceKey = "control-room.recent-documents.v1"
  private static let recoveredDraftPreferenceKey = "control-room.recovered-draft.v1"

  enum Connection: Equatable {
    case disconnected
    case connecting
    case live
    case failed(String)
  }

  @Published var endpoint: String {
    didSet { preferences.set(endpoint, forKey: Self.endpointPreferenceKey) }
  }
  @Published private(set) var connection: Connection = .disconnected
  @Published private(set) var snapshot: GraphSnapshot?
  @Published private(set) var catalog = ControlCatalog()
  @Published var workingNodes: [GraphNode] = []
  @Published var selectedNodeID: String?
  @Published private(set) var dirty = false
  @Published var notice: String?
  @Published private(set) var applying = false
  @Published private(set) var hasRevisionConflict = false
  @Published private(set) var currentDocumentURL: URL?
  @Published private(set) var recentDocuments: [URL] = []
  @Published private(set) var recoveredDraftAvailable = false
  @Published private(set) var rackSnapshot: RackSnapshot?
  @Published private(set) var rackProposal: RackProposal?
  @Published private(set) var rackBusy = false
  @Published private(set) var rackCanUndo = false
  @Published var rackNotice: String?

  private let preferences: UserDefaults
  private var editingBaseRevision: UInt64?
  private var socket: URLSessionWebSocketTask?
  private var socketTask: Task<Void, Never>?
  private var reconnectTask: Task<Void, Never>?
  private var undoStack: [[GraphNode]] = []
  private var redoStack: [[GraphNode]] = []
  private var recoveredDraft: RecoveredDraft?
  private let documentStore = PatchDocumentStore()

  init(preferences: UserDefaults = .standard) {
    self.preferences = preferences
    endpoint =
      preferences.string(forKey: Self.endpointPreferenceKey) ?? "http://127.0.0.1:7777"
    recentDocuments =
      (preferences.stringArray(forKey: Self.recentDocumentsPreferenceKey) ?? []).map {
        URL(fileURLWithPath: $0)
      }
    if let data = preferences.data(forKey: Self.recoveredDraftPreferenceKey) {
      recoveredDraft = try? JSONDecoder().decode(RecoveredDraft.self, from: data)
    }
  }

  var meters: [String: Double] {
    Dictionary(uniqueKeysWithValues: (snapshot?.meters ?? []).map { ($0.id, $0.value) })
  }

  var revision: UInt64 { snapshot?.revision ?? 0 }
  var isRunning: Bool { snapshot?.runtimeRunning ?? false }
  var canUndo: Bool { !undoStack.isEmpty }
  var canRedo: Bool { !redoStack.isEmpty }
  var validationErrors: [String] { validateDraft(workingNodes, catalog: catalog) }

  func connect() {
    disconnect(scheduleReconnect: false)
    connection = .connecting
    notice = nil
    Task { await refreshAndOpenSocket() }
  }

  func disconnect(scheduleReconnect: Bool = false) {
    socketTask?.cancel()
    socketTask = nil
    socket?.cancel(with: .goingAway, reason: nil)
    socket = nil
    if !scheduleReconnect {
      reconnectTask?.cancel()
      reconnectTask = nil
    }
    connection = .disconnected
  }

  func refresh() {
    Task { await refreshState() }
  }

  func refreshRack() {
    Task { await refreshRackSnapshot() }
  }

  func rejectRackProposal() {
    rackProposal = nil
    rackNotice = nil
  }

  func proposeRackPatch(_ prompt: String, provider: String) {
    let trimmed = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else {
      rackNotice = "Describe the Rack patch you want to build."
      return
    }
    Task { await sendRackProposal(trimmed, provider: provider) }
  }

  func applyRackProposal() {
    guard let rackProposal else { return }
    Task { await sendRackApply(rackProposal.id) }
  }

  func undoRackPatch() {
    Task { await sendRackUndo() }
  }

  func markDirty() {
    if !dirty {
      editingBaseRevision = snapshot?.revision
      hasRevisionConflict = false
    }
    dirty = true
    if !hasRevisionConflict { notice = nil }
    persistRecovery()
  }

  func discardChanges() {
    guard let snapshot else { return }
    workingNodes = snapshot.graph.nodes
    selectedNodeID = workingNodes.first?.id
    dirty = false
    editingBaseRevision = nil
    hasRevisionConflict = false
    undoStack.removeAll()
    redoStack.removeAll()
    clearRecovery()
    notice = "Local changes discarded"
  }

  func add(_ kind: NodeKind) {
    add(kind.fallbackModule)
  }

  func add(_ module: ModuleCatalogEntry) {
    recordHistory()
    let base = module.id
    var number = 1
    var id = "\(base)-\(number)"
    while workingNodes.contains(where: { $0.id == id }) {
      number += 1
      id = "\(base)-\(number)"
    }
    workingNodes.append(GraphNode(id: id, spec: module.defaultSpec, input: nil))
    selectedNodeID = id
    markDirty()
  }

  func remove(_ id: String) {
    guard workingNodes.contains(where: { $0.id == id }) else { return }
    recordHistory()
    workingNodes.removeAll { $0.id == id }
    for index in workingNodes.indices {
      if workingNodes[index].input == id { workingNodes[index].input = nil }
      if case .takeover(let control, let epsilon) = workingNodes[index].spec, control == id {
        workingNodes[index].spec = .takeover(control: "", epsilon: epsilon)
      }
      if case .envelope(let gate, let attack, let decay, let sustain, let release) =
        workingNodes[index].spec, gate == id
      {
        workingNodes[index].spec = .envelope(
          gate: "", attack: attack, decay: decay, sustain: sustain, release: release)
      }
    }
    if selectedNodeID == id { selectedNodeID = workingNodes.first?.id }
    markDirty()
  }

  func updateNode(_ node: GraphNode, replacing oldID: String) {
    guard !node.id.trimmingCharacters(in: .whitespaces).isEmpty else {
      notice = "Node IDs cannot be empty"
      return
    }
    guard !workingNodes.contains(where: { $0.id == node.id && $0.id != oldID }) else {
      notice = "Node IDs must be unique"
      return
    }
    guard let index = workingNodes.firstIndex(where: { $0.id == oldID }) else { return }
    guard workingNodes[index] != node else { return }
    recordHistory()
    workingNodes[index] = node
    if node.id != oldID {
      for candidate in workingNodes.indices {
        if workingNodes[candidate].input == oldID { workingNodes[candidate].input = node.id }
        if case .takeover(let control, let epsilon) = workingNodes[candidate].spec, control == oldID
        {
          workingNodes[candidate].spec = .takeover(control: node.id, epsilon: epsilon)
        }
        if case .envelope(let gate, let attack, let decay, let sustain, let release) =
          workingNodes[candidate].spec, gate == oldID
        {
          workingNodes[candidate].spec = .envelope(
            gate: node.id, attack: attack, decay: decay, sustain: sustain, release: release)
        }
      }
      selectedNodeID = node.id
    }
    markDirty()
  }

  func patch(source sourceID: String, target targetID: String, portID: String) {
    guard sourceID != targetID,
      workingNodes.contains(where: {
        $0.id == sourceID && $0.spec.patchPorts.contains(where: { $0.direction == .output })
      }),
      var target = workingNodes.first(where: { $0.id == targetID })
    else {
      notice = "That cable cannot be connected"
      return
    }
    switch (portID, target.spec) {
    case ("signal", _):
      target.input = sourceID
    case ("gate", .envelope(_, let attack, let decay, let sustain, let release)):
      target.spec = .envelope(
        gate: sourceID, attack: attack, decay: decay, sustain: sustain, release: release)
    case ("control", .takeover(_, let epsilon)):
      target.spec = .takeover(control: sourceID, epsilon: epsilon)
    default:
      notice = "That module does not expose a \(portID) input"
      return
    }
    updateNode(target, replacing: targetID)
    notice = "Patched \(sourceID) OUT → \(targetID) \(portID.uppercased())"
  }

  func disconnect(target targetID: String, portID: String) {
    guard var target = workingNodes.first(where: { $0.id == targetID }) else { return }
    switch (portID, target.spec) {
    case ("signal", _):
      guard target.input != nil else { return }
      target.input = nil
    case ("gate", .envelope(let gate, let attack, let decay, let sustain, let release)):
      guard !gate.isEmpty else { return }
      target.spec = .envelope(
        gate: "", attack: attack, decay: decay, sustain: sustain, release: release)
    case ("control", .takeover(let control, let epsilon)):
      guard !control.isEmpty else { return }
      target.spec = .takeover(control: "", epsilon: epsilon)
    default: return
    }
    updateNode(target, replacing: targetID)
    notice = "Disconnected \(targetID) \(portID.uppercased())"
  }

  func apply() {
    guard let snapshot, dirty, !applying, !hasRevisionConflict, validationErrors.isEmpty else {
      if !validationErrors.isEmpty { notice = validationErrors[0] }
      return
    }
    applying = true
    notice = nil
    let oldNodes = snapshot.graph.nodes
    let newNodes = workingNodes
    var commands = oldNodes.map { GraphCommand.removeNode(id: $0.id) }
    commands.append(contentsOf: newNodes.map { .addNode(id: $0.id, spec: $0.spec) })
    commands.append(
      contentsOf: newNodes.compactMap { node in
        node.input.map { .connect(source: $0, target: node.id) }
      })
    let batch = CommandBatch(
      baseRevision: editingBaseRevision ?? snapshot.revision, commands: commands)
    Task { await send(batch) }
  }

  func undo() {
    guard let previous = undoStack.popLast() else { return }
    redoStack.append(workingNodes)
    workingNodes = previous
    reconcileDraftAfterHistoryChange(message: "Undid local edit")
  }

  func redo() {
    guard let next = redoStack.popLast() else { return }
    undoStack.append(workingNodes)
    workingNodes = next
    reconcileDraftAfterHistoryChange(message: "Redid local edit")
  }

  func newPatch() {
    guard confirmReplacingDraft() else { return }
    recordHistory()
    currentDocumentURL = nil
    workingNodes = []
    selectedNodeID = nil
    markDirty()
    notice = "New empty patch — add a source to begin"
  }

  func openPatch() {
    guard confirmReplacingDraft() else { return }
    let panel = NSOpenPanel()
    panel.allowedContentTypes = [UTType(filenameExtension: "controlroompatch") ?? .json]
    panel.allowsMultipleSelection = false
    panel.canChooseDirectories = false
    guard panel.runModal() == .OK, let url = panel.url else { return }
    do {
      try loadDocument(from: url)
    } catch {
      notice = "Could not open patch: \(error.localizedDescription)"
    }
  }

  func openRecent(_ url: URL) {
    guard confirmReplacingDraft() else { return }
    do {
      try loadDocument(from: url)
    } catch {
      removeRecent(url)
      notice = "Could not open patch: \(error.localizedDescription)"
    }
  }

  func savePatch() {
    if let currentDocumentURL {
      do {
        try saveDocument(to: currentDocumentURL)
      } catch {
        notice = "Could not save patch: \(error.localizedDescription)"
      }
    } else {
      savePatchAs()
    }
  }

  func savePatchAs() {
    let panel = NSSavePanel()
    panel.allowedContentTypes = [UTType(filenameExtension: "controlroompatch") ?? .json]
    panel.nameFieldStringValue =
      currentDocumentURL?.lastPathComponent ?? "Untitled.controlroompatch"
    guard panel.runModal() == .OK, let url = panel.url else { return }
    do {
      try saveDocument(to: url)
    } catch {
      notice = "Could not save patch: \(error.localizedDescription)"
    }
  }

  func loadDocument(from url: URL) throws {
    let document = try documentStore.load(from: url)
    currentDocumentURL = url
    undoStack.removeAll()
    redoStack.removeAll()
    workingNodes = document.graph.nodes
    selectedNodeID = workingNodes.first?.id
    editingBaseRevision = snapshot?.revision
    dirty = workingNodes != snapshot?.graph.nodes
    hasRevisionConflict = false
    addRecent(url)
    if dirty { persistRecovery() } else { clearRecovery() }
    notice = "Opened \(document.name)"
  }

  func saveDocument(to url: URL) throws {
    let name = url.deletingPathExtension().lastPathComponent
    try documentStore.save(
      PatchDocument(name: name, graph: ControlGraph(nodes: workingNodes)), to: url)
    currentDocumentURL = url
    addRecent(url)
    notice = "Saved \(url.lastPathComponent)"
  }

  func restoreRecoveredDraft() {
    guard let recoveredDraft, recoveredDraft.endpoint == endpoint else { return }
    recordHistory()
    workingNodes = recoveredDraft.nodes
    selectedNodeID = workingNodes.first?.id
    editingBaseRevision = snapshot?.revision
    dirty = workingNodes != snapshot?.graph.nodes
    hasRevisionConflict = false
    recoveredDraftAvailable = false
    if dirty { persistRecovery() } else { clearRecovery() }
    notice = "Recovered draft restored locally — review before Apply"
  }

  func dismissRecoveredDraft() {
    clearRecovery()
    notice = "Recovered draft dismissed"
  }

  private func recordHistory() {
    if undoStack.last != workingNodes { undoStack.append(workingNodes) }
    if undoStack.count > 100 { undoStack.removeFirst(undoStack.count - 100) }
    redoStack.removeAll()
  }

  private func reconcileDraftAfterHistoryChange(message: String) {
    if selectedNodeID == nil || !workingNodes.contains(where: { $0.id == selectedNodeID }) {
      selectedNodeID = workingNodes.first?.id
    }
    dirty = workingNodes != snapshot?.graph.nodes
    if dirty {
      editingBaseRevision = editingBaseRevision ?? snapshot?.revision
      if let base = editingBaseRevision, snapshot?.revision != base { hasRevisionConflict = true }
    } else {
      editingBaseRevision = nil
      hasRevisionConflict = false
    }
    if dirty { persistRecovery() } else { clearRecovery() }
    notice = message
  }

  private func confirmReplacingDraft() -> Bool {
    guard dirty else { return true }
    let alert = NSAlert()
    alert.messageText = "Replace unapplied changes?"
    alert.informativeText = "The current local draft has not been applied to the engine."
    alert.addButton(withTitle: "Replace Draft")
    alert.addButton(withTitle: "Cancel")
    return alert.runModal() == .alertFirstButtonReturn
  }

  private func addRecent(_ url: URL) {
    recentDocuments.removeAll { $0.standardizedFileURL == url.standardizedFileURL }
    recentDocuments.insert(url, at: 0)
    if recentDocuments.count > 10 { recentDocuments.removeLast(recentDocuments.count - 10) }
    preferences.set(
      recentDocuments.map(\.path), forKey: Self.recentDocumentsPreferenceKey)
  }

  private func removeRecent(_ url: URL) {
    recentDocuments.removeAll { $0.standardizedFileURL == url.standardizedFileURL }
    preferences.set(
      recentDocuments.map(\.path), forKey: Self.recentDocumentsPreferenceKey)
  }

  private func persistRecovery() {
    guard dirty else {
      clearRecovery()
      return
    }
    let recovery = RecoveredDraft(
      endpoint: endpoint, baseRevision: editingBaseRevision ?? snapshot?.revision ?? 0,
      nodes: workingNodes)
    recoveredDraft = recovery
    if let data = try? JSONEncoder().encode(recovery) {
      preferences.set(data, forKey: Self.recoveredDraftPreferenceKey)
    }
  }

  private func clearRecovery() {
    recoveredDraft = nil
    recoveredDraftAvailable = false
    preferences.removeObject(forKey: Self.recoveredDraftPreferenceKey)
  }

  private func normalizedBaseURL() -> URL? {
    URL(
      string: endpoint.trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(
        in: CharacterSet(charactersIn: "/")))
  }

  private func refreshAndOpenSocket() async {
    guard await refreshState() else {
      scheduleReconnect()
      return
    }
    await refreshCatalog()
    await refreshRackSnapshot()
    openSocket()
  }

  private func refreshRackSnapshot() async {
    guard let base = normalizedBaseURL() else { return }
    do {
      let (data, response) = try await URLSession.shared.data(
        from: base.appending(path: "api/v1/rack/snapshot"))
      try Self.requireSuccess(response, data: data)
      rackSnapshot = try JSONDecoder().decode(RackSnapshot.self, from: data)
    } catch {
      rackSnapshot = nil
    }
  }

  private func sendRackProposal(_ prompt: String, provider: String) async {
    guard let base = normalizedBaseURL(), !rackBusy else { return }
    rackBusy = true
    rackProposal = nil
    rackNotice = nil
    defer { rackBusy = false }
    do {
      var request = URLRequest(url: base.appending(path: "api/v1/rack/copilot/propose"))
      request.httpMethod = "POST"
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      request.httpBody = try JSONEncoder().encode(
        RackProposePayload(prompt: prompt, provider: provider == "default" ? nil : provider))
      let (data, response) = try await URLSession.shared.data(for: request)
      try Self.requireSuccess(response, data: data)
      rackProposal = try JSONDecoder().decode(RackProposal.self, from: data)
      rackNotice = "Validated against the live Rack inventory. Review before applying."
      await refreshRackSnapshot()
    } catch {
      rackNotice = error.localizedDescription
      await refreshRackSnapshot()
    }
  }

  private func sendRackApply(_ proposalID: String) async {
    guard let base = normalizedBaseURL(), !rackBusy else { return }
    rackBusy = true
    defer { rackBusy = false }
    do {
      var request = URLRequest(url: base.appending(path: "api/v1/rack/copilot/apply"))
      request.httpMethod = "POST"
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      request.httpBody = try JSONEncoder().encode(RackApplyPayload(proposalID: proposalID))
      let (data, response) = try await URLSession.shared.data(for: request)
      try Self.requireSuccess(response, data: data)
      let result = try JSONDecoder().decode(RackMutationResponse.self, from: data)
      rackProposal = nil
      rackCanUndo = true
      rackNotice = result.message
      await refreshRackSnapshot()
    } catch {
      rackNotice = error.localizedDescription
    }
  }

  private func sendRackUndo() async {
    guard let base = normalizedBaseURL(), !rackBusy else { return }
    rackBusy = true
    defer { rackBusy = false }
    do {
      var request = URLRequest(url: base.appending(path: "api/v1/rack/copilot/undo"))
      request.httpMethod = "POST"
      let (data, response) = try await URLSession.shared.data(for: request)
      try Self.requireSuccess(response, data: data)
      let result = try JSONDecoder().decode(RackMutationResponse.self, from: data)
      rackCanUndo = false
      rackNotice = result.message
      await refreshRackSnapshot()
    } catch {
      rackNotice = error.localizedDescription
    }
  }

  private func refreshCatalog() async {
    guard let base = normalizedBaseURL() else { return }
    do {
      let (data, response) = try await URLSession.shared.data(
        from: base.appending(path: "api/v1/catalog"))
      try Self.requireSuccess(response, data: data)
      catalog = try JSONDecoder().decode(ControlCatalog.self, from: data)
    } catch {
      notice = "Catalog unavailable: \(error.localizedDescription)"
    }
  }

  @discardableResult
  private func refreshState() async -> Bool {
    guard let base = normalizedBaseURL() else {
      connection = .failed("Invalid server address")
      notice = "Connection failed: invalid server address"
      return false
    }
    do {
      let (data, response) = try await URLSession.shared.data(
        from: base.appending(path: "api/v1/state"))
      try Self.requireSuccess(response, data: data)
      accept(try JSONDecoder().decode(GraphSnapshot.self, from: data))
      return true
    } catch {
      connection = .failed(error.localizedDescription)
      notice = "Connection failed: \(error.localizedDescription)"
      return false
    }
  }

  private func openSocket() {
    guard let base = normalizedBaseURL(),
      var parts = URLComponents(url: base, resolvingAgainstBaseURL: false)
    else { return }
    parts.scheme = parts.scheme == "https" ? "wss" : "ws"
    parts.path = "/api/v1/events"
    guard let url = parts.url else { return }
    let socket = URLSession.shared.webSocketTask(with: url)
    self.socket = socket
    socket.resume()
    connection = .live
    socketTask = Task { [weak self] in
      guard let self else { return }
      do {
        while !Task.isCancelled {
          let message = try await socket.receive()
          let data: Data
          switch message {
          case .data(let value): data = value
          case .string(let value): data = Data(value.utf8)
          @unknown default: continue
          }
          if let event = try? JSONDecoder().decode(ServerEventEnvelope.self, from: data),
            event.type == "snapshot", let snapshot = event.payload
          {
            self.accept(snapshot)
          }
        }
      } catch {
        guard !Task.isCancelled else { return }
        self.connection = .failed(error.localizedDescription)
        self.notice = "Connection lost: \(error.localizedDescription)"
        self.scheduleReconnect()
      }
    }
  }

  private func scheduleReconnect() {
    reconnectTask?.cancel()
    reconnectTask = Task { [weak self] in
      try? await Task.sleep(for: .seconds(1.25))
      guard !Task.isCancelled else { return }
      self?.connect()
    }
  }

  func accept(_ incoming: GraphSnapshot) {
    if dirty, !applying, let editingBaseRevision, incoming.revision != editingBaseRevision {
      hasRevisionConflict = true
      notice =
        "Revision conflict: the server moved from \(editingBaseRevision) to \(incoming.revision). Discard local edits to reload it."
    }
    snapshot = incoming
    connection = .live
    if !dirty {
      let serverGraphChanged = workingNodes != incoming.graph.nodes
      workingNodes = incoming.graph.nodes
      if serverGraphChanged {
        undoStack.removeAll()
        redoStack.removeAll()
      }
      if selectedNodeID == nil || !workingNodes.contains(where: { $0.id == selectedNodeID }) {
        selectedNodeID = workingNodes.first?.id
      }
      if let recoveredDraft, recoveredDraft.endpoint == endpoint,
        recoveredDraft.nodes != incoming.graph.nodes
      {
        recoveredDraftAvailable = true
      }
    }
    if let warning = incoming.persistenceWarning { notice = warning }
  }

  private func send(_ batch: CommandBatch) async {
    defer { applying = false }
    guard let base = normalizedBaseURL() else { return }
    do {
      var request = URLRequest(url: base.appending(path: "api/v1/commands"))
      request.httpMethod = "POST"
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      request.httpBody = try JSONEncoder().encode(batch)
      let (data, response) = try await URLSession.shared.data(for: request)
      try Self.requireSuccess(response, data: data)
      let result = try JSONDecoder().decode(CommandResponse.self, from: data)
      dirty = false
      editingBaseRevision = nil
      hasRevisionConflict = false
      undoStack.removeAll()
      redoStack.removeAll()
      clearRecovery()
      notice = result.warning ?? (result.persisted ? "Patch applied and saved" : "Patch applied")
      _ = await refreshState()
    } catch {
      if (error as NSError).code == 409 {
        hasRevisionConflict = true
        notice = "Revision conflict: reload the server patch before applying this draft."
        _ = await refreshState()
      } else {
        notice = error.localizedDescription
      }
    }
  }

  nonisolated private static func requireSuccess(_ response: URLResponse, data: Data) throws {
    guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode)
    else {
      let message =
        (try? JSONDecoder().decode(APIErrorPayload.self, from: data).error)
        ?? "Control Room request failed"
      throw NSError(
        domain: "ControlRoom", code: (response as? HTTPURLResponse)?.statusCode ?? -1,
        userInfo: [NSLocalizedDescriptionKey: message])
    }
  }
}
