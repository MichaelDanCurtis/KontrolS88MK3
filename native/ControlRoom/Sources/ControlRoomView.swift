import SwiftUI

struct ControlRoomView: View {
  @ObservedObject var client: ControlRoomClient

  var body: some View {
    HStack(spacing: 0) {
      Sidebar(client: client)
        .frame(width: 238)
      Rectangle().fill(CRTheme.line).frame(width: 1)
      VStack(spacing: 0) {
        TopBar(client: client)
        Rectangle().fill(CRTheme.line).frame(height: 1)
        SignalCanvas(client: client)
      }
      Rectangle().fill(CRTheme.line).frame(width: 1)
      StudioPanel(client: client)
        .frame(width: 340)
    }
    .foregroundStyle(CRTheme.primary)
    .background(CRTheme.canvas)
  }
}

private struct Sidebar: View {
  @ObservedObject var client: ControlRoomClient

  private var ioRows: [IOStatus] {
    if let io = client.snapshot?.io, !io.isEmpty { return io }
    return client.catalog.devices.map {
      IOStatus(
        id: $0.id, label: $0.displayName, transport: "midi_out", endpoint: $0.port,
        state: .disconnected, connectAttempts: 0, reconnects: 0, received: 0, sent: 0,
        errors: 0, dropped: 0)
    }
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: 11) {
        ZStack {
          RoundedRectangle(cornerRadius: 10).fill(
            LinearGradient(
              colors: [CRTheme.amber, .orange], startPoint: .topLeading, endPoint: .bottomTrailing)
          )
          Image(systemName: "waveform.path.ecg").font(.system(size: 16, weight: .black))
            .foregroundStyle(.black)
        }
        .frame(width: 36, height: 36)
        VStack(alignment: .leading, spacing: 1) {
          Text("CONTROL ROOM").font(.system(size: 13, weight: .black, design: .rounded)).tracking(
            1.2)
          Text("SIGNAL WORKSTATION").font(.system(size: 8, weight: .bold)).tracking(1.6)
            .foregroundStyle(CRTheme.secondary)
        }
      }
      .padding(.horizontal, 18).padding(.top, 22).padding(.bottom, 24)

      sidebarLabel("ENGINE")
      connectionCard
        .padding(.horizontal, 12).padding(.bottom, 20)

      sidebarLabel("PATCH")
      Label("Signal Field", systemImage: "point.3.connected.trianglepath.dotted")
        .font(.system(size: 12, weight: .semibold))
        .padding(.horizontal, 16).padding(.vertical, 11)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(CRTheme.amber.opacity(0.10))
        .overlay(alignment: .leading) { Rectangle().fill(CRTheme.amber).frame(width: 2) }

      sidebarLabel("I/O STATUS").padding(.top, 18)
      if ioRows.isEmpty {
        Text("No configured adapters").font(.system(size: 11)).foregroundStyle(CRTheme.secondary)
          .padding(16)
      } else {
        ForEach(ioRows) { status in
          HStack(spacing: 9) {
            Circle().fill(statusColor(status.state)).frame(
              width: 6, height: 6
            )
            .shadow(color: statusColor(status.state), radius: status.state == .connected ? 5 : 0)
            VStack(alignment: .leading, spacing: 2) {
              Text(status.label.uppercased()).font(.system(size: 11, weight: .bold)).tracking(0.7)
              Text(
                "\(status.transport.replacingOccurrences(of: "_", with: " ").uppercased()) · \(status.state.rawValue.replacingOccurrences(of: "_", with: " "))"
              ).font(.system(size: 9)).foregroundStyle(CRTheme.secondary)
              if status.errors > 0 || status.dropped > 0 {
                Text("\(status.errors) ERR · \(status.dropped) DROP")
                  .font(.system(size: 8, weight: .bold, design: .monospaced))
                  .foregroundStyle(CRTheme.coral)
              }
            }
            Spacer()
            if status.reconnects > 0 {
              Text("R\(status.reconnects)").font(
                .system(size: 8, weight: .bold, design: .monospaced)
              )
              .foregroundStyle(CRTheme.amber)
            }
          }.padding(.horizontal, 17).padding(.vertical, 9)
        }
      }
      Spacer()

      VStack(alignment: .leading, spacing: 8) {
        TextField("Server", text: $client.endpoint)
          .textFieldStyle(.plain).font(.system(size: 10, design: .monospaced))
          .padding(9).background(.black.opacity(0.22)).clipShape(RoundedRectangle(cornerRadius: 7))
          .onSubmit { client.connect() }
        Button {
          client.connect()
        } label: {
          Label("Reconnect", systemImage: "arrow.clockwise").frame(maxWidth: .infinity)
        }
        .buttonStyle(.bordered).controlSize(.small)
      }.padding(14)
    }
    .background(CRTheme.panel)
  }

  private var connectionCard: some View {
    HStack(spacing: 10) {
      ZStack {
        Circle().fill(connectionColor.opacity(0.15)).frame(width: 30, height: 30)
        Circle().fill(connectionColor).frame(width: 7, height: 7).shadow(
          color: connectionColor, radius: 7)
      }
      VStack(alignment: .leading, spacing: 2) {
        Text(connectionTitle).font(.system(size: 11, weight: .bold))
        Text(connectionDetail)
          .font(.system(size: 9)).foregroundStyle(CRTheme.secondary)
      }
      Spacer()
    }
    .padding(11).background(CRTheme.raised).clipShape(RoundedRectangle(cornerRadius: 11))
    .overlay(RoundedRectangle(cornerRadius: 11).stroke(CRTheme.line))
  }

  private var connectionColor: Color {
    switch client.connection {
    case .live: CRTheme.mint
    case .connecting: CRTheme.amber
    case .failed: CRTheme.coral
    case .disconnected: CRTheme.secondary
    }
  }
  private var connectionTitle: String {
    switch client.connection {
    case .live: "LIVE ENGINE"
    case .connecting: "CONNECTING"
    case .failed: "OFFLINE"
    case .disconnected: "DISCONNECTED"
    }
  }
  private func statusColor(_ state: IOConnectionState) -> Color {
    switch state {
    case .connected: CRTheme.mint
    case .dryRun: CRTheme.cyan
    case .disconnected: CRTheme.coral
    case .disabled: CRTheme.secondary
    }
  }
  private var connectionDetail: String {
    switch client.connection {
    case .failed(let message): message
    default: client.isRunning ? "1 kHz engine online" : "Waiting for runtime"
    }
  }
  private func sidebarLabel(_ title: String) -> some View {
    Text(title).font(.system(size: 9, weight: .bold)).tracking(1.6).foregroundStyle(
      CRTheme.secondary
    )
    .padding(.horizontal, 17).padding(.bottom, 7)
  }
}

private struct TopBar: View {
  @ObservedObject var client: ControlRoomClient

  var body: some View {
    HStack(spacing: 10) {
      VStack(alignment: .leading, spacing: 2) {
        Text("SIGNAL FIELD").font(.system(size: 14, weight: .bold, design: .rounded)).tracking(0.6)
          .lineLimit(1)
        Text(
          "\(client.currentDocumentURL?.deletingPathExtension().lastPathComponent.uppercased() ?? "LIVE PATCH")  ·  REV \(client.revision)  ·  \(client.workingNodes.count) MODULES"
        )
        .font(.system(size: 9, weight: .semibold, design: .monospaced)).foregroundStyle(
          CRTheme.secondary
        ).lineLimit(1)
      }
      .frame(minWidth: 100, maxWidth: 150, alignment: .leading).layoutPriority(1)
      Spacer()
      metric("TICK", client.snapshot?.runtimeTick.formatted() ?? "—", CRTheme.cyan)
      metric("MIDI", client.snapshot?.limiter.sentMessages.formatted() ?? "—", CRTheme.amber)
      metric("DROPS", client.snapshot?.limiter.busRateLimited.formatted() ?? "—", CRTheme.coral)
      Divider().frame(height: 28).overlay(CRTheme.line)
      if client.dirty {
        Label(
          client.hasRevisionConflict ? "CONFLICT" : "UNAPPLIED",
          systemImage: client.hasRevisionConflict ? "exclamationmark.triangle.fill" : "circle.fill"
        ).font(.system(size: 9, weight: .bold))
          .foregroundStyle(client.hasRevisionConflict ? CRTheme.coral : CRTheme.amber)
      }
      if !client.validationErrors.isEmpty {
        Label("\(client.validationErrors.count) ISSUES", systemImage: "exclamationmark.circle.fill")
          .font(.system(size: 9, weight: .bold)).foregroundStyle(CRTheme.coral)
      }
      Button {
        client.undo()
      } label: {
        Image(systemName: "arrow.uturn.backward")
      }.buttonStyle(.plain).foregroundStyle(CRTheme.secondary).disabled(!client.canUndo)
        .help("Undo local edit (⌘Z)")
      Button {
        client.redo()
      } label: {
        Image(systemName: "arrow.uturn.forward")
      }.buttonStyle(.plain).foregroundStyle(CRTheme.secondary).disabled(!client.canRedo)
        .help("Redo local edit (⇧⌘Z)")
      Button {
        client.discardChanges()
      } label: {
        Image(systemName: "trash")
      }
      .buttonStyle(.plain).foregroundStyle(CRTheme.secondary).disabled(!client.dirty)
      .help("Discard local changes")
      Button {
        client.apply()
      } label: {
        HStack(spacing: 7) {
          if client.applying {
            ProgressView().controlSize(.small)
          } else {
            Image(systemName: "bolt.fill")
          }
          Text("APPLY")
        }.font(.system(size: 10, weight: .black)).tracking(0.7).padding(.horizontal, 10).padding(
          .vertical, 9)
      }
      .buttonStyle(.plain).foregroundStyle(.black)
      .background(
        client.hasRevisionConflict || !client.validationErrors.isEmpty
          ? CRTheme.coral.opacity(0.55)
          : (client.dirty ? CRTheme.amber : CRTheme.secondary.opacity(0.35))
      )
      .clipShape(RoundedRectangle(cornerRadius: 8))
      .shadow(color: client.dirty ? CRTheme.amber.opacity(0.25) : .clear, radius: 10)
      .disabled(
        !client.dirty || client.applying || client.hasRevisionConflict
          || !client.validationErrors.isEmpty)
    }
    .padding(.horizontal, 12).frame(height: 65).background(CRTheme.panel)
  }

  private func metric(_ name: String, _ value: String, _ color: Color) -> some View {
    VStack(alignment: .trailing, spacing: 1) {
      Text(name).font(.system(size: 8, weight: .bold)).tracking(1.3).foregroundStyle(
        CRTheme.secondary)
      Text(value).font(.system(size: 12, weight: .semibold, design: .monospaced)).foregroundStyle(
        color)
    }.frame(minWidth: 38, alignment: .trailing)
  }
}

private struct SignalCanvas: View {
  @ObservedObject var client: ControlRoomClient
  @State private var positions: [String: CGPoint] = [:]
  @State private var dragOrigins: [String: CGPoint] = [:]
  @State private var showingLibrary = false
  @State private var pendingSourceID: String?
  private let layoutStore = CanvasLayoutStore()

  private let cardSize = CGSize(width: 232, height: 150)
  private let fieldSize = CGSize(width: 1600, height: 1000)

  var body: some View {
    ZStack(alignment: .topLeading) {
      ScrollView([.horizontal, .vertical]) {
        ZStack(alignment: .topLeading) {
          GridBackground()
          SignalWires(nodes: client.workingNodes, positions: positions, cardSize: cardSize)
          ForEach(client.workingNodes) { node in
            NodeCard(
              node: node,
              value: client.meters[node.id] ?? 0,
              selected: client.selectedNodeID == node.id,
              pendingOutput: pendingSourceID == node.id,
              onSelect: { client.selectedNodeID = node.id },
              onDelete: {
                if pendingSourceID == node.id { pendingSourceID = nil }
                client.remove(node.id)
              },
              onPortTap: { handlePortTap(node: node, port: $0) }
            )
            .frame(width: cardSize.width, height: cardSize.height)
            .position(cardCenter(for: node.id))
            .gesture(dragGesture(for: node.id))
          }
        }
        .frame(width: fieldSize.width, height: fieldSize.height)
      }
      .scrollIndicators(.hidden)

      Button {
        showingLibrary.toggle()
      } label: {
        HStack(spacing: 10) {
          ZStack {
            Circle().fill(CRTheme.cyan.opacity(0.16))
            Image(systemName: "square.grid.3x3.fill").foregroundStyle(CRTheme.cyan)
          }.frame(width: 30, height: 30)
          VStack(alignment: .leading, spacing: 1) {
            Text("MODULE LIBRARY").font(.system(size: 9, weight: .black)).tracking(1.1)
            Text("\(moduleLibrary.count) modules · typed patch points")
              .font(.system(size: 9)).foregroundStyle(CRTheme.secondary)
          }
          Image(systemName: "chevron.down").font(.system(size: 9, weight: .bold))
            .foregroundStyle(CRTheme.secondary)
        }.padding(10)
      }
      .buttonStyle(.plain).glassPanel(radius: 17).padding(16)
      .popover(isPresented: $showingLibrary, arrowEdge: .top) {
        ModuleLibrary(modules: moduleLibrary) { module in
          client.add(module)
          seedMissingPositions()
          showingLibrary = false
        }
      }

      if client.recoveredDraftAvailable {
        HStack(spacing: 10) {
          Image(systemName: "clock.arrow.circlepath").foregroundStyle(CRTheme.amber)
          VStack(alignment: .leading, spacing: 2) {
            Text("RECOVERED DRAFT").font(.system(size: 9, weight: .black)).tracking(1.2)
            Text("Review it locally before Apply").font(.system(size: 9)).foregroundStyle(
              CRTheme.secondary)
          }
          Button("Restore") { client.restoreRecoveredDraft() }.buttonStyle(.borderedProminent)
            .tint(CRTheme.amber).foregroundStyle(.black).controlSize(.small)
          Button("Ignore") { client.dismissRecoveredDraft() }.buttonStyle(.plain)
            .foregroundStyle(CRTheme.secondary)
        }.padding(12).glassPanel(radius: 12)
          .frame(maxWidth: .infinity, alignment: .topTrailing).padding(16)
      }

      if let notice = client.notice {
        Text(notice).font(.system(size: 11, weight: .medium)).padding(.horizontal, 13).padding(
          .vertical, 9
        )
        .background(CRTheme.raised).clipShape(Capsule()).overlay(
          Capsule().stroke(CRTheme.amber.opacity(0.35))
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom).padding(.bottom, 18)
      }
    }
    .background(CRTheme.canvas)
    .onAppear { restorePositions() }
    .onChange(of: client.workingNodes.map(\.id)) { _, _ in seedMissingPositions() }
    .onChange(of: client.endpoint) { _, _ in restorePositions() }
  }

  private var moduleLibrary: [ModuleCatalogEntry] {
    client.catalog.modules.isEmpty
      ? NodeKind.allCases.map(\.fallbackModule) : client.catalog.modules
  }

  private func restorePositions() {
    positions = layoutStore.load(endpoint: client.endpoint).mapValues {
      CGPoint(x: $0.x, y: $0.y)
    }
    seedMissingPositions()
  }

  private func savePositions() {
    let stored = positions.mapValues { CanvasPosition(x: $0.x, y: $0.y) }
    layoutStore.save(stored, endpoint: client.endpoint)
  }

  private func seedMissingPositions() {
    for (index, node) in client.workingNodes.enumerated() where positions[node.id] == nil {
      positions[node.id] = CGPoint(
        x: 90 + CGFloat(index % 4) * 290, y: 130 + CGFloat(index / 4) * 220)
    }
  }

  private func cardCenter(for id: String) -> CGPoint {
    let origin = positions[id] ?? CGPoint(x: 90, y: 130)
    return CGPoint(x: origin.x + cardSize.width / 2, y: origin.y + cardSize.height / 2)
  }

  private func dragGesture(for id: String) -> some Gesture {
    DragGesture()
      .onChanged { value in
        let origin = dragOrigins[id] ?? positions[id] ?? .zero
        dragOrigins[id] = origin
        positions[id] = CGPoint(
          x: max(20, origin.x + value.translation.width),
          y: max(90, origin.y + value.translation.height))
      }
      .onEnded { _ in
        dragOrigins[id] = nil
        savePositions()
      }
  }

  private func handlePortTap(node: GraphNode, port: ModulePort) {
    if port.direction == .output {
      pendingSourceID = pendingSourceID == node.id ? nil : node.id
      client.notice =
        pendingSourceID == nil
        ? "Cable cancelled" : "Patching from \(node.id) OUT — choose an input jack"
      return
    }
    if let sourceID = pendingSourceID {
      client.patch(source: sourceID, target: node.id, portID: port.id)
      pendingSourceID = nil
    } else {
      client.disconnect(target: node.id, portID: port.id)
    }
  }
}

private struct SignalWires: View {
  let nodes: [GraphNode]
  let positions: [String: CGPoint]
  let cardSize: CGSize

  var body: some View {
    Canvas { context, _ in
      for node in nodes {
        if let sourceID = node.input {
          draw(
            from: sourceID, to: node.id, color: CRTheme.tint(for: node.spec.kind), dashed: false,
            targetOffsetY: 52, context: &context)
        }
        if case .takeover(let control, _) = node.spec, !control.isEmpty {
          draw(
            from: control, to: node.id, color: CRTheme.coral, dashed: true,
            targetOffsetY: 103, context: &context)
        }
        if case .envelope(let gate, _, _, _, _) = node.spec, !gate.isEmpty {
          draw(
            from: gate, to: node.id, color: CRTheme.amber, dashed: true,
            targetOffsetY: 52, context: &context)
        }
      }
    }
  }

  private func draw(
    from sourceID: String, to targetID: String, color: Color, dashed: Bool,
    targetOffsetY: CGFloat, context: inout GraphicsContext
  ) {
    guard let source = positions[sourceID], let target = positions[targetID] else { return }
    let start = CGPoint(x: source.x + cardSize.width, y: source.y + 52)
    let end = CGPoint(x: target.x, y: target.y + targetOffsetY)
    let bend = max(70, abs(end.x - start.x) * 0.45)
    var path = Path()
    path.move(to: start)
    path.addCurve(
      to: end, control1: CGPoint(x: start.x + bend, y: start.y),
      control2: CGPoint(x: end.x - bend, y: end.y))
    context.stroke(
      path, with: .color(color.opacity(0.15)), style: StrokeStyle(lineWidth: 8, lineCap: .round))
    context.stroke(
      path, with: .color(color.opacity(0.82)),
      style: StrokeStyle(lineWidth: 1.8, lineCap: .round, dash: dashed ? [5, 5] : []))
  }
}

private struct NodeCard: View {
  let node: GraphNode
  let value: Double
  let selected: Bool
  let pendingOutput: Bool
  let onSelect: () -> Void
  let onDelete: () -> Void
  let onPortTap: (ModulePort) -> Void
  private var tint: Color { CRTheme.tint(for: node.spec.kind) }
  private var normalized: Double { min(1, max(0, abs(value) <= 1 ? value : value / 127)) }

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 10) {
        ZStack {
          RoundedRectangle(cornerRadius: 7).fill(tint.opacity(0.16))
          Image(systemName: node.spec.kind.systemImage).foregroundStyle(tint)
        }
        .frame(width: 31, height: 31)
        VStack(alignment: .leading, spacing: 2) {
          Text(node.spec.kind.rawValue.uppercased()).font(.system(size: 8, weight: .black))
            .tracking(1.3).foregroundStyle(tint)
          Text(node.id).font(.system(size: 13, weight: .bold, design: .rounded)).lineLimit(1)
        }
        Spacer()
        Menu {
          Button("Remove", role: .destructive, action: onDelete)
        } label: {
          Image(systemName: "ellipsis").foregroundStyle(CRTheme.secondary)
        }.menuStyle(.borderlessButton).fixedSize()
      }.padding(12)
      Rectangle().fill(CRTheme.line).frame(height: 1)
      VStack(alignment: .leading, spacing: 11) {
        Text(node.spec.subtitle).font(.system(size: 10)).foregroundStyle(CRTheme.secondary)
          .lineLimit(1)
        HStack(alignment: .lastTextBaseline) {
          Text(value.formatted(.number.precision(.fractionLength(3)))).font(
            .system(size: 23, weight: .light, design: .rounded)
          ).contentTransition(.numericText())
          Spacer()
          Circle().fill(tint).frame(width: 5, height: 5).shadow(color: tint, radius: 6)
        }
        GeometryReader { geometry in
          ZStack(alignment: .leading) {
            Capsule().fill(.white.opacity(0.06))
            Capsule().fill(
              LinearGradient(
                colors: [tint.opacity(0.55), tint], startPoint: .leading, endPoint: .trailing)
            ).frame(width: geometry.size.width * normalized)
          }
        }.frame(height: 4)
      }.padding(12)
    }
    .background(CRTheme.raised.opacity(0.95)).clipShape(
      RoundedRectangle(cornerRadius: 14, style: .continuous)
    )
    .overlay(
      RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(
        selected ? tint.opacity(0.9) : CRTheme.line, lineWidth: selected ? 1.5 : 1)
    )
    .shadow(
      color: selected ? tint.opacity(0.14) : .black.opacity(0.25), radius: selected ? 18 : 8, y: 4
    )
    .overlay {
      NodePatchPorts(
        node: node, value: value, tint: tint, pendingOutput: pendingOutput, onTap: onPortTap)
    }
    .contentShape(Rectangle()).onTapGesture(perform: onSelect)
  }
}

private struct NodePatchPorts: View {
  let node: GraphNode
  let value: Double
  let tint: Color
  let pendingOutput: Bool
  let onTap: (ModulePort) -> Void

  var body: some View {
    GeometryReader { geometry in
      ForEach(Array(node.spec.patchPorts.enumerated()), id: \.element.id) { index, port in
        let inputIndex = node.spec.patchPorts.prefix(index).filter { $0.direction == .input }.count
        let y = port.direction == .output ? 52.0 : 52.0 + Double(inputIndex) * 51.0
        let color = port.signal == .gate ? CRTheme.amber : tint
        let connected = isConnected(port)
        Button {
          onTap(port)
        } label: {
          HStack(spacing: 4) {
            if port.direction == .output {
              Text(port.name).font(.system(size: 7, weight: .black, design: .monospaced))
                .tracking(0.7).foregroundStyle(CRTheme.secondary)
            }
            ZStack {
              Circle().fill(Color.black.opacity(0.9)).frame(width: 18, height: 18)
                .overlay(Circle().stroke(color.opacity(0.85), lineWidth: 2))
              Circle()
                .fill(
                  connected || (port.direction == .output && pendingOutput)
                    ? color : color.opacity(0.18)
                ).frame(width: 7, height: 7)
                .shadow(
                  color: connected || (port.direction == .output && pendingOutput)
                    ? color.opacity(0.8) : .clear, radius: 5)
            }
            if port.direction == .input {
              Text(port.name).font(.system(size: 7, weight: .black, design: .monospaced))
                .tracking(0.7).foregroundStyle(connected ? color : CRTheme.secondary)
            }
          }
        }.buttonStyle(.plain)
          .position(
            x: port.direction == .input ? 8 : geometry.size.width - 8,
            y: CGFloat(y)
          )
          .accessibilityLabel(
            "\(port.name) \(port.direction.rawValue) \(connected ? "connected" : "unpatched")")
      }
    }
  }

  private func isConnected(_ port: ModulePort) -> Bool {
    if port.direction == .output { return value.isFinite }
    switch port.id {
    case "signal": return node.input != nil
    case "gate":
      if case .envelope(let gate, _, _, _, _) = node.spec { return !gate.isEmpty }
      return false
    case "control":
      if case .takeover(let control, _) = node.spec { return !control.isEmpty }
      return false
    default: return false
    }
  }
}

private struct ModuleLibrary: View {
  let modules: [ModuleCatalogEntry]
  let onAdd: (ModuleCatalogEntry) -> Void
  @State private var query = ""
  @State private var category = "All"

  private var categories: [String] {
    ["All"] + Set(modules.map(\.category)).sorted()
  }

  private var filtered: [ModuleCatalogEntry] {
    modules.filter { module in
      let categoryMatches = category == "All" || module.category == category
      let needle = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
      return categoryMatches
        && (needle.isEmpty || module.name.lowercased().contains(needle)
          || module.summary.lowercased().contains(needle)
          || module.category.lowercased().contains(needle))
    }
  }

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 10) {
        Image(systemName: "magnifyingglass").foregroundStyle(CRTheme.secondary)
        TextField("Search modules", text: $query).textFieldStyle(.plain)
        Text("\(filtered.count)").font(.system(size: 10, weight: .bold, design: .monospaced))
          .foregroundStyle(CRTheme.cyan)
      }.padding(13).background(CRTheme.raised)
      ScrollView(.horizontal) {
        HStack(spacing: 6) {
          ForEach(categories, id: \.self) { item in
            Button(item) { category = item }.buttonStyle(.plain)
              .font(.system(size: 9, weight: .bold)).padding(.horizontal, 10).padding(.vertical, 6)
              .background(category == item ? CRTheme.cyan.opacity(0.18) : CRTheme.raised)
              .foregroundStyle(category == item ? CRTheme.cyan : CRTheme.secondary)
              .clipShape(Capsule()).overlay(Capsule().stroke(CRTheme.line))
          }
        }.padding(10)
      }.scrollIndicators(.hidden)
      Divider().overlay(CRTheme.line)
      ScrollView {
        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
          ForEach(filtered) { module in
            Button {
              onAdd(module)
            } label: {
              HStack(alignment: .top, spacing: 10) {
                ZStack {
                  RoundedRectangle(cornerRadius: 9).fill(
                    CRTheme.tint(for: module.defaultSpec.kind).opacity(0.14))
                  Image(systemName: module.defaultSpec.kind.systemImage)
                    .foregroundStyle(CRTheme.tint(for: module.defaultSpec.kind))
                }.frame(width: 35, height: 35)
                VStack(alignment: .leading, spacing: 3) {
                  Text(module.name).font(.system(size: 11, weight: .bold)).lineLimit(1)
                  Text(module.summary).font(.system(size: 9)).foregroundStyle(CRTheme.secondary)
                    .lineLimit(2)
                  HStack(spacing: 5) {
                    Text(module.category.uppercased()).font(.system(size: 7, weight: .black))
                      .tracking(0.7)
                    Spacer()
                    ForEach(module.ports) { port in
                      Circle().fill(
                        port.signal == .gate
                          ? CRTheme.amber : CRTheme.tint(for: module.defaultSpec.kind)
                      ).frame(width: 5, height: 5)
                    }
                  }.foregroundStyle(CRTheme.secondary)
                }
              }.padding(10).frame(maxWidth: .infinity, minHeight: 78, alignment: .topLeading)
            }.buttonStyle(.plain).background(CRTheme.raised.opacity(0.72))
              .clipShape(RoundedRectangle(cornerRadius: 12)).overlay(
                RoundedRectangle(cornerRadius: 12).stroke(CRTheme.line))
          }
        }.padding(10)
      }
    }.frame(width: 560, height: 620).background(CRTheme.panel)
  }
}

private struct StudioPanel: View {
  @ObservedObject var client: ControlRoomClient
  @State private var page = 0

  var body: some View {
    VStack(spacing: 0) {
      Picker("Detail", selection: $page) {
        Label("Module", systemImage: "slider.horizontal.3").tag(0)
        Label("Rack Copilot", systemImage: "sparkles").tag(1)
      }
      .pickerStyle(.segmented).padding(12)
      Rectangle().fill(CRTheme.line).frame(height: 1)
      if page == 0 { Inspector(client: client) } else { RackCopilotPanel(client: client) }
    }.background(CRTheme.panel)
  }
}

private struct RackCopilotPanel: View {
  @ObservedObject var client: ControlRoomClient
  @State private var prompt = ""
  @State private var provider = "default"

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 14) {
        HStack(alignment: .top) {
          VStack(alignment: .leading, spacing: 3) {
            Text("RACK COPILOT").font(.system(size: 9, weight: .black)).tracking(1.6)
              .foregroundStyle(CRTheme.mint)
            Text("Build with intent").font(.system(size: 19, weight: .bold, design: .rounded))
          }
          Spacer()
          Circle().fill(client.rackSnapshot == nil ? CRTheme.coral : CRTheme.mint).frame(
            width: 8, height: 8
          )
          .shadow(color: client.rackSnapshot == nil ? CRTheme.coral : CRTheme.mint, radius: 7)
          .padding(.top, 5)
        }
        Text(
          "Copilot inspects the installed VCV library and current patch, then stages a constrained, reversible command plan."
        )
        .font(.system(size: 11)).foregroundStyle(CRTheme.secondary).fixedSize(
          horizontal: false, vertical: true)

        HStack {
          Image(systemName: "shippingbox")
          if let rack = client.rackSnapshot {
            Text("\(rack.patch.modules.count) PATCHED  ·  \(rack.catalog.count) MODELS")
          } else {
            Text("ADD BRIDGE 8 TO AN OPEN RACK PATCH")
          }
          Spacer()
          Button {
            client.refreshRack()
          } label: {
            Image(systemName: "arrow.clockwise")
          }
          .buttonStyle(.plain)
        }
        .font(.system(size: 9, weight: .bold, design: .monospaced))
        .foregroundStyle(client.rackSnapshot == nil ? CRTheme.amber : CRTheme.mint)
        .padding(10).background(CRTheme.raised).clipShape(RoundedRectangle(cornerRadius: 9))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(CRTheme.line))

        Text("AI PLANNER").font(.system(size: 9, weight: .black)).tracking(1.3)
          .foregroundStyle(CRTheme.secondary)
        Picker("AI planner", selection: $provider) {
          Text("DEFAULT").tag("default")
          Text("OPENAI").tag("openai")
          Text("CLAUDE").tag("claude")
          Text("GROK").tag("grok")
          Text("ACP").tag("acp")
        }
        .labelsHidden().pickerStyle(.segmented)

        Text("PATCH REQUEST").font(.system(size: 9, weight: .black)).tracking(1.3)
          .foregroundStyle(CRTheme.secondary)
        TextEditor(text: $prompt).font(.system(size: 11)).scrollContentBackground(.hidden)
          .padding(8).frame(minHeight: 100).background(.black.opacity(0.24))
          .clipShape(RoundedRectangle(cornerRadius: 9))
          .overlay(RoundedRectangle(cornerRadius: 9).stroke(CRTheme.line))
        HStack(spacing: 6) {
          promptButton(
            "GENERATIVE", "Add a clocked generative melody with quantization and slow modulation.")
          promptButton(
            "SYNTH", "Build a playable subtractive synth voice from my installed modules.")
          promptButton("MOTION", "Add tasteful movement to this patch without replacing anything.")
        }
        Button {
          client.proposeRackPatch(prompt, provider: provider)
        } label: {
          HStack {
            Spacer()
            if client.rackBusy { ProgressView().controlSize(.small) }
            Text(client.rackBusy ? "PLANNING…" : "DRAFT PATCH PLAN")
            Spacer()
          }
        }
        .buttonStyle(.borderedProminent).tint(CRTheme.mint).foregroundStyle(.black)
        .disabled(client.rackBusy || client.rackSnapshot == nil)
        .opacity(client.rackBusy || client.rackSnapshot == nil ? 0.42 : 1)

        if let notice = client.rackNotice {
          Text(notice).font(.system(size: 10)).foregroundStyle(
            client.rackProposal == nil && !client.rackCanUndo ? CRTheme.coral : CRTheme.secondary
          ).padding(9).frame(maxWidth: .infinity, alignment: .leading)
            .background(CRTheme.raised).clipShape(RoundedRectangle(cornerRadius: 8))
        }

        if let proposal = client.rackProposal {
          proposalCard(proposal)
        }

        Button {
          client.undoRackPatch()
        } label: {
          Label("Undo last Copilot patch", systemImage: "arrow.uturn.backward").frame(
            maxWidth: .infinity)
        }
        .buttonStyle(.bordered).disabled(!client.rackCanUndo || client.rackBusy)

        Text(
          "Copilot never installs, purchases, or patches anything until Apply. A stale patch is rejected, and native Rack history remains authoritative."
        )
        .font(.system(size: 9)).foregroundStyle(CRTheme.secondary).fixedSize(
          horizontal: false, vertical: true)
      }.padding(16)
    }
  }

  private func promptButton(_ title: String, _ value: String) -> some View {
    Button(title) { prompt = value }.buttonStyle(.bordered).controlSize(.mini)
      .font(.system(size: 8, weight: .bold, design: .monospaced))
  }

  private func proposalCard(_ proposal: RackProposal) -> some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack {
        Text("STAGED PROPOSAL").font(.system(size: 8, weight: .black)).tracking(1.2)
        Spacer()
        Text("\((proposal.provider ?? "ai").uppercased()) · \(proposal.model)")
          .font(.system(size: 8, design: .monospaced)).foregroundStyle(CRTheme.secondary)
      }.foregroundStyle(CRTheme.mint)
      Text(proposal.summary).font(.system(size: 13, weight: .bold)).fixedSize(
        horizontal: false, vertical: true)
      ForEach(Array(proposal.warnings.enumerated()), id: \.offset) { _, warning in
        Label(warning, systemImage: "exclamationmark.triangle.fill").font(.system(size: 9))
          .foregroundStyle(CRTheme.amber)
      }
      ForEach(proposal.commands) { command in
        VStack(alignment: .leading, spacing: 3) {
          Text(command.type.replacingOccurrences(of: "_", with: " ").uppercased())
            .font(.system(size: 8, weight: .black, design: .monospaced)).foregroundStyle(
              CRTheme.mint)
          Text(command.label).font(.system(size: 10)).fixedSize(horizontal: false, vertical: true)
        }.padding(.vertical, 6).frame(maxWidth: .infinity, alignment: .leading)
          .overlay(alignment: .bottom) { Rectangle().fill(CRTheme.line).frame(height: 1) }
      }
      if !proposal.librarySuggestions.isEmpty {
        Text("LIBRARY IDEAS — NOT INSTALLED").font(.system(size: 8, weight: .black))
          .foregroundStyle(CRTheme.amber)
        ForEach(Array(proposal.librarySuggestions.enumerated()), id: \.offset) { _, suggestion in
          VStack(alignment: .leading, spacing: 2) {
            Text(suggestion.query).font(.system(size: 10, weight: .bold))
            Text(suggestion.reason).font(.system(size: 9)).foregroundStyle(CRTheme.secondary)
          }
        }
      }
      HStack {
        Button("Reject") { client.rejectRackProposal() }.buttonStyle(.bordered)
        Button("Apply in Rack") { client.applyRackProposal() }.buttonStyle(.borderedProminent)
          .tint(CRTheme.mint).foregroundStyle(.black).disabled(client.rackBusy)
      }.frame(maxWidth: .infinity, alignment: .trailing)
    }.padding(12).background(CRTheme.raised).clipShape(RoundedRectangle(cornerRadius: 12))
      .overlay(RoundedRectangle(cornerRadius: 12).stroke(CRTheme.mint.opacity(0.28)))
  }
}

private struct Inspector: View {
  @ObservedObject var client: ControlRoomClient

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack {
        Text("MODULE").font(.system(size: 10, weight: .black)).tracking(1.8)
        Spacer()
        Image(systemName: "slider.horizontal.3")
      }
      .foregroundStyle(CRTheme.secondary).padding(18)
      Rectangle().fill(CRTheme.line).frame(height: 1)
      if !client.validationErrors.isEmpty {
        VStack(alignment: .leading, spacing: 5) {
          Label("PATCH NEEDS ATTENTION", systemImage: "exclamationmark.triangle.fill")
            .font(.system(size: 9, weight: .black)).tracking(1.1).foregroundStyle(CRTheme.coral)
          ForEach(Array(client.validationErrors.prefix(3).enumerated()), id: \.offset) { _, error in
            Text(error).font(.system(size: 10)).foregroundStyle(CRTheme.secondary)
          }
        }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
          .background(CRTheme.coral.opacity(0.07))
      }
      if let id = client.selectedNodeID,
        let node = client.workingNodes.first(where: { $0.id == id })
      {
        NodeEditor(
          node: node, allNodes: client.workingNodes, catalog: client.catalog,
          onChange: { client.updateNode($0, replacing: id) }, onDelete: { client.remove(id) }
        )
        .id(id)
      } else {
        VStack(spacing: 12) {
          Image(systemName: "cursorarrow.click.2").font(.system(size: 28)).foregroundStyle(
            CRTheme.secondary)
          Text("Select a module").font(.system(size: 13, weight: .semibold))
          Text("Its routing and semantic parameters will appear here.").font(.system(size: 11))
            .foregroundStyle(CRTheme.secondary).multilineTextAlignment(.center)
        }.frame(maxWidth: .infinity, maxHeight: .infinity).padding(30)
      }
    }.background(CRTheme.panel)
  }
}

private struct NodeEditor: View {
  let sourceNode: GraphNode
  @State private var node: GraphNode
  let allNodes: [GraphNode]
  let catalog: ControlCatalog
  let onChange: (GraphNode) -> Void
  let onDelete: () -> Void
  private var candidates: [GraphNode] { allNodes.filter { $0.id != node.id } }

  init(
    node: GraphNode, allNodes: [GraphNode], catalog: ControlCatalog,
    onChange: @escaping (GraphNode) -> Void, onDelete: @escaping () -> Void
  ) {
    sourceNode = node
    _node = State(initialValue: node)
    self.allNodes = allNodes
    self.catalog = catalog
    self.onChange = onChange
    self.onDelete = onDelete
  }

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 18) {
        HStack(spacing: 12) {
          ZStack {
            RoundedRectangle(cornerRadius: 11).fill(CRTheme.tint(for: node.spec.kind).opacity(0.15))
            Image(systemName: node.spec.kind.systemImage).foregroundStyle(
              CRTheme.tint(for: node.spec.kind))
          }
          .frame(width: 42, height: 42)
          VStack(alignment: .leading, spacing: 2) {
            Text(node.spec.kind.rawValue.uppercased()).font(.system(size: 9, weight: .black))
              .tracking(1.3).foregroundStyle(CRTheme.tint(for: node.spec.kind))
            Text(node.id).font(.system(size: 16, weight: .bold, design: .rounded)).lineLimit(1)
          }
        }
        section("IDENTITY") { field("Node ID", text: bind(\.id)) }
        if node.spec.kind != .lfo && node.spec.kind != .sampleHold
          && node.spec.kind != .stepSequence && node.spec.kind != .envelope
          && node.spec.kind != .macro && node.spec.kind != .externalSource
        {
          section("ROUTING") {
            Picker("Input", selection: bindOptional(\.input)) {
              Text("None").tag(String?.none)
              ForEach(candidates) { Text($0.id).tag(String?.some($0.id)) }
            }.pickerStyle(.menu)
          }
        }
        specEditor
        Divider().overlay(CRTheme.line)
        Button(role: .destructive, action: onDelete) {
          Label("Remove Module", systemImage: "trash").frame(maxWidth: .infinity)
        }
        .buttonStyle(.bordered).tint(CRTheme.coral)
      }.padding(18)
    }.onChange(of: sourceNode) { _, updated in
      if updated != node { node = updated }
    }
  }

  @ViewBuilder private var specEditor: some View {
    switch node.spec {
    case .lfo(let shape, let frequency):
      section("OSCILLATOR") {
        Picker(
          "Shape",
          selection: Binding(
            get: { shape }, set: { setSpec(.lfo(shape: $0, frequency: frequency)) })
        ) {
          Text("Triangle").tag("triangle")
          Text("Sine").tag("sine")
        }.pickerStyle(.segmented).tint(CRTheme.tint(for: node.spec.kind))
        numberField(
          "Frequency",
          value: Binding(get: { frequency }, set: { setSpec(.lfo(shape: shape, frequency: $0)) }),
          suffix: "Hz", range: 0.001...50)
      }
    case .sampleHold(let rate, let seed):
      section("RANDOM SOURCE") {
        numberField(
          "Rate",
          value: Binding(get: { rate }, set: { setSpec(.sampleHold(rate: $0, seed: seed)) }),
          suffix: "Hz", range: 0.001...50)
        seedField(
          value: Binding(get: { seed }, set: { setSpec(.sampleHold(rate: rate, seed: $0)) }))
        Text("The same seed and absolute timeline always produce the same pattern.")
          .font(.system(size: 9)).foregroundStyle(CRTheme.secondary)
      }
    case .stepSequence(let rate, let values):
      section("STEP SEQUENCE") {
        numberField(
          "Rate",
          value: Binding(
            get: { rate }, set: { setSpec(.stepSequence(rate: $0, values: values)) }),
          suffix: "steps/s", range: 0.001...50)
        ForEach(Array(values.indices), id: \.self) { index in
          HStack(spacing: 8) {
            Text("\(index + 1)").font(.system(size: 9, design: .monospaced))
              .foregroundStyle(CRTheme.secondary).frame(width: 18, alignment: .trailing)
            Slider(
              value: Binding(
                get: { values[index] },
                set: { next in
                  var edited = values
                  edited[index] = next
                  setSpec(.stepSequence(rate: rate, values: edited))
                }), in: 0...1
            ).tint(CRTheme.tint(for: node.spec.kind))
            Text(values[index].formatted(.number.precision(.fractionLength(2))))
              .font(.system(size: 9, design: .monospaced)).frame(width: 34, alignment: .trailing)
            Button {
              var edited = values
              edited.remove(at: index)
              setSpec(.stepSequence(rate: rate, values: edited))
            } label: {
              Image(systemName: "minus.circle")
            }
            .buttonStyle(.plain).disabled(values.count <= 1)
          }
        }
        Button {
          guard values.count < 64 else { return }
          setSpec(.stepSequence(rate: rate, values: values + [values.last ?? 0]))
        } label: {
          Label("Add Step", systemImage: "plus")
        }
        .buttonStyle(.bordered).disabled(values.count >= 64)
      }
    case .slew(let rise, let fall):
      section("SLEW LIMITER") {
        numberField(
          "Rise",
          value: Binding(get: { rise }, set: { setSpec(.slew(rise: $0, fall: fall)) }),
          suffix: "/s", range: 0.001...100)
        numberField(
          "Fall",
          value: Binding(get: { fall }, set: { setSpec(.slew(rise: rise, fall: $0)) }),
          suffix: "/s", range: 0.001...100)
      }
    case .clamp(let minimum, let maximum):
      section("CLAMP") {
        numberField(
          "Minimum",
          value: Binding(get: { minimum }, set: { setSpec(.clamp(min: $0, max: maximum)) }),
          range: -10000...10000)
        numberField(
          "Maximum",
          value: Binding(get: { maximum }, set: { setSpec(.clamp(min: minimum, max: $0)) }),
          range: -10000...10000)
      }
    case .curve(let exponent):
      section("CURVE") {
        numberField(
          "Exponent",
          value: Binding(get: { exponent }, set: { setSpec(.curve(exponent: $0)) }),
          range: 0.05...8)
        Text(
          exponent < 1
            ? "Expands lower values" : exponent > 1 ? "Compresses lower values" : "Linear"
        )
        .font(.system(size: 9)).foregroundStyle(CRTheme.secondary)
      }
    case .quantize(let steps):
      section("QUANTIZE") {
        Stepper(
          "\(steps) levels",
          value: Binding(get: { steps }, set: { setSpec(.quantize(steps: $0)) }), in: 2...128)
        Text("Snaps a 0…1 signal onto evenly spaced levels.")
          .font(.system(size: 9)).foregroundStyle(CRTheme.secondary)
      }
    case .math(let operation, let operand):
      section("ARITHMETIC") {
        Picker(
          "Operation",
          selection: Binding(
            get: { operation }, set: { setSpec(.math(operation: $0, operand: operand)) })
        ) {
          Text("Add").tag("add")
          Text("Subtract").tag("subtract")
          Text("Multiply").tag("multiply")
          Text("Divide").tag("divide")
        }.pickerStyle(.menu)
        numberField(
          "Operand",
          value: Binding(
            get: { operand }, set: { setSpec(.math(operation: operation, operand: $0)) }),
          range: -1000...1000)
      }
    case .envelope(let gate, let attack, let decay, let sustain, let release):
      section("ADSR ENVELOPE") {
        Picker(
          "Gate",
          selection: Binding(
            get: { gate },
            set: {
              setSpec(
                .envelope(
                  gate: $0, attack: attack, decay: decay, sustain: sustain, release: release))
            })
        ) {
          Text("Choose gate").tag("")
          ForEach(candidates) { Text($0.id).tag($0.id) }
        }.pickerStyle(.menu)
        numberField(
          "Attack",
          value: Binding(
            get: { attack },
            set: {
              setSpec(
                .envelope(
                  gate: gate, attack: $0, decay: decay, sustain: sustain, release: release))
            }), suffix: "s", range: 0...10)
        numberField(
          "Decay",
          value: Binding(
            get: { decay },
            set: {
              setSpec(
                .envelope(
                  gate: gate, attack: attack, decay: $0, sustain: sustain, release: release))
            }), suffix: "s", range: 0...10)
        numberField(
          "Sustain",
          value: Binding(
            get: { sustain },
            set: {
              setSpec(
                .envelope(
                  gate: gate, attack: attack, decay: decay, sustain: $0, release: release))
            }), range: 0...1)
        numberField(
          "Release",
          value: Binding(
            get: { release },
            set: {
              setSpec(
                .envelope(
                  gate: gate, attack: attack, decay: decay, sustain: sustain, release: $0))
            }), suffix: "s", range: 0...10)
      }
    case .macro(let binding, let defaultValue):
      section("MACRO CONTROL") {
        bindingEditor(
          label: "Binding", entries: catalog.externalInputs,
          selection: Binding(
            get: { binding },
            set: { setSpec(.macro(binding: $0, defaultValue: defaultValue)) }))
        numberField(
          "Default",
          value: Binding(
            get: { defaultValue },
            set: { setSpec(.macro(binding: binding, defaultValue: $0)) }),
          range: -10000...10000)
      }
    case .scale(let minimum, let maximum):
      section("RANGE") {
        numberField(
          "Minimum",
          value: Binding(get: { minimum }, set: { setSpec(.scale(min: $0, max: maximum)) }),
          range: -10000...10000)
        numberField(
          "Maximum",
          value: Binding(get: { maximum }, set: { setSpec(.scale(min: minimum, max: $0)) }),
          range: -10000...10000)
      }
    case .sink(let device, let parameter):
      section("SEMANTIC OUTPUT") {
        if catalog.devices.isEmpty {
          field(
            "Device",
            text: Binding(
              get: { device }, set: { setSpec(.sink(device: $0, parameter: parameter)) }))
          field(
            "Parameter",
            text: Binding(
              get: { parameter }, set: { setSpec(.sink(device: device, parameter: $0)) }))
        } else {
          Picker(
            "Device",
            selection: Binding(
              get: { device },
              set: { selected in
                let first = catalog.devices.first(where: { $0.id == selected })?.parameters.first?
                  .id
                setSpec(.sink(device: selected, parameter: first ?? parameter))
              })
          ) {
            if !catalog.devices.contains(where: { $0.id == device }) {
              Text("\(device) (unavailable)").tag(device)
            }
            ForEach(catalog.devices) { entry in
              Text("\(entry.displayName) · \(entry.id)").tag(entry.id)
            }
          }.pickerStyle(.menu)

          let parameters = catalog.devices.first(where: { $0.id == device })?.parameters ?? []
          Picker(
            "Parameter",
            selection: Binding(
              get: { parameter }, set: { setSpec(.sink(device: device, parameter: $0)) })
          ) {
            if !parameters.contains(where: { $0.id == parameter }) {
              Text("\(parameter) (unavailable)").tag(parameter)
            }
            ForEach(parameters) { entry in
              Text("\(entry.id) · \(entry.encoding.uppercased())").tag(entry.id)
            }
          }.pickerStyle(.menu)
          if let selected = parameters.first(where: { $0.id == parameter }) {
            Text(
              "Range \(selected.min.formatted())…\(selected.max.formatted()) · \(selected.encoding.uppercased())"
            ).font(.system(size: 9, design: .monospaced)).foregroundStyle(CRTheme.secondary)
          }
        }
      }
    case .externalSource(let binding, let defaultValue):
      section("CONTROL INPUT") {
        bindingEditor(
          label: "Binding", entries: catalog.externalInputs,
          selection: Binding(
            get: { binding },
            set: { setSpec(.externalSource(binding: $0, defaultValue: defaultValue)) }))
        numberField(
          "Default",
          value: Binding(
            get: { defaultValue },
            set: { setSpec(.externalSource(binding: binding, defaultValue: $0)) }),
          range: -10000...10000)
      }
    case .externalSink(let binding):
      section("SIGNAL OUTPUT") {
        bindingEditor(
          label: "Binding", entries: catalog.externalOutputs,
          selection: Binding(
            get: { binding }, set: { setSpec(.externalSink(binding: $0)) }))
      }
    case .takeover(let control, let epsilon):
      section("PICKUP") {
        Picker(
          "Physical control",
          selection: Binding(
            get: { control }, set: { setSpec(.takeover(control: $0, epsilon: epsilon)) })
        ) {
          Text("Choose control").tag("")
          ForEach(candidates) { Text($0.id).tag($0.id) }
        }.pickerStyle(.menu)
        numberField(
          "Tolerance",
          value: Binding(
            get: { epsilon }, set: { setSpec(.takeover(control: control, epsilon: $0)) }),
          range: 0...1)
      }
    }
  }

  private func setSpec(_ spec: NodeSpec) {
    node.spec = spec
    onChange(node)
  }
  private func bind(_ keyPath: WritableKeyPath<GraphNode, String>) -> Binding<String> {
    Binding(
      get: { node[keyPath: keyPath] },
      set: {
        node[keyPath: keyPath] = $0
        onChange(node)
      })
  }
  private func bindOptional(_ keyPath: WritableKeyPath<GraphNode, String?>) -> Binding<String?> {
    Binding(
      get: { node[keyPath: keyPath] },
      set: {
        node[keyPath: keyPath] = $0
        onChange(node)
      })
  }
  private func field(_ label: String, text: Binding<String>) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(label).font(.system(size: 10)).foregroundStyle(CRTheme.secondary)
      TextField(label, text: text).textFieldStyle(.plain).padding(9).background(
        .black.opacity(0.22)
      ).clipShape(RoundedRectangle(cornerRadius: 7)).font(.system(size: 11, design: .monospaced))
    }
  }
  private func numberField(
    _ label: String, value: Binding<Double>, suffix: String = "", range: ClosedRange<Double>
  ) -> some View {
    VStack(alignment: .leading, spacing: 7) {
      HStack {
        Text(label).font(.system(size: 10)).foregroundStyle(CRTheme.secondary)
        Spacer()
        TextField(label, value: value, format: .number.precision(.fractionLength(0...3)))
          .textFieldStyle(.plain).multilineTextAlignment(.trailing)
          .font(.system(size: 10, design: .monospaced)).frame(width: 64)
        if !suffix.isEmpty { Text(suffix).font(.system(size: 10, design: .monospaced)) }
      }
      Slider(value: value, in: range).tint(CRTheme.tint(for: node.spec.kind))
    }
  }
  private func seedField(value: Binding<UInt64>) -> some View {
    HStack {
      Text("Seed").font(.system(size: 10)).foregroundStyle(CRTheme.secondary)
      Spacer()
      TextField("Seed", value: value, format: .number.grouping(.never))
        .textFieldStyle(.plain).multilineTextAlignment(.trailing)
        .font(.system(size: 10, design: .monospaced)).frame(width: 120)
    }
  }
  @ViewBuilder private func bindingEditor(
    label: String, entries: [BindingCatalogEntry], selection: Binding<String>
  ) -> some View {
    if entries.isEmpty {
      field(label, text: selection)
    } else {
      Picker(label, selection: selection) {
        if !entries.contains(where: { $0.id == selection.wrappedValue }) {
          Text("\(selection.wrappedValue) (unavailable)").tag(selection.wrappedValue)
        }
        ForEach(entries) { entry in
          Text("\(entry.id) · \(entry.transport.uppercased())").tag(entry.id)
        }
      }.pickerStyle(.menu)
    }
  }
  private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content)
    -> some View
  {
    VStack(alignment: .leading, spacing: 12) {
      Text(title).font(.system(size: 9, weight: .black)).tracking(1.5).foregroundStyle(
        CRTheme.secondary)
      content()
    }
  }
}
