import SwiftUI

@main
struct ControlRoomNativeApp: App {
  @StateObject private var client = ControlRoomClient()

  var body: some Scene {
    WindowGroup {
      ControlRoomView(client: client)
        .preferredColorScheme(.dark)
        .frame(minWidth: 1180, minHeight: 760)
        .onAppear { client.connect() }
    }
    .windowStyle(.hiddenTitleBar)
    .windowToolbarStyle(.unifiedCompact(showsTitle: false))
    .commands {
      CommandGroup(replacing: .newItem) {
        Button("New Patch") { client.newPatch() }
          .keyboardShortcut("n", modifiers: [.command])
        Button("Open Patch…") { client.openPatch() }
          .keyboardShortcut("o", modifiers: [.command])
        Menu("Open Recent") {
          if client.recentDocuments.isEmpty {
            Text("No Recent Patches")
          } else {
            ForEach(client.recentDocuments, id: \.self) { url in
              Button(url.lastPathComponent) { client.openRecent(url) }
            }
          }
        }
        Divider()
        Button("Save Patch") { client.savePatch() }
          .keyboardShortcut("s", modifiers: [.command])
        Button("Save Patch As…") { client.savePatchAs() }
          .keyboardShortcut("s", modifiers: [.command, .shift])
      }
      CommandGroup(replacing: .undoRedo) {
        Button("Undo") { client.undo() }
          .keyboardShortcut("z", modifiers: [.command])
          .disabled(!client.canUndo)
        Button("Redo") { client.redo() }
          .keyboardShortcut("z", modifiers: [.command, .shift])
          .disabled(!client.canRedo)
      }
      CommandMenu("Patch") {
        Button("Apply Changes") { client.apply() }
          .keyboardShortcut(.return, modifiers: [.command])
          .disabled(!client.dirty || client.hasRevisionConflict || !client.validationErrors.isEmpty)
        Button("Discard Changes") { client.discardChanges() }
          .keyboardShortcut(.delete, modifiers: [.command])
          .disabled(!client.dirty)
        Divider()
        Button("Reconnect") { client.connect() }
          .keyboardShortcut("r", modifiers: [.command, .shift])
      }
    }
  }
}
