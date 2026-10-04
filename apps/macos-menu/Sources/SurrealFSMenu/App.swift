import SwiftUI
import AppKit

@main
struct SurrealFSMenuApp: App {
    @StateObject private var viewModel = AppViewModel()
    @State private var searchPanel: NSPanel?
    @State private var preferencesWindow: NSWindow?
    
    var body: some Scene {
        MenuBarExtra {
            MenuContentView(
                viewModel: viewModel,
                onQuickSearch: {
                    toggleSearchPanel()
                },
                onPreferences: {
                    openPreferencesWindow()
                },
                onQuit: {
                    NSApplication.shared.terminate(nil)
                }
            )
        } label: {
            HStack(spacing: 4) {
                Image(systemName: "cylinder.split.1x2.fill")
                Circle()
                    .fill(viewModel.statusColor)
                    .frame(width: 6, height: 6)
            }
        }
        .menuBarExtraStyle(.window)
    }
    
    private func toggleSearchPanel() {
        if let panel = searchPanel, panel.isVisible {
            panel.orderOut(nil)
            return
        }
        
        if searchPanel == nil {
            let panel = NSPanel(
                contentRect: NSRect(x: 0, y: 0, width: 580, height: 400),
                styleMask: [.nonactivatingPanel, .titled, .fullSizeContentView],
                backing: .buffered,
                defer: false
            )
            panel.isFloatingPanel = true
            panel.level = .floating
            panel.titleVisibility = .hidden
            panel.titlebarAppearsTransparent = true
            panel.isOpaque = false
            panel.backgroundColor = .clear
            panel.center()
            
            let contentView = SpotlightSearchView(
                mountPath: viewModel.mountInfo.mountPath,
                onClose: { [weak panel] in
                    panel?.orderOut(nil)
                }
            )
            panel.contentView = NSHostingView(rootView: contentView)
            self.searchPanel = panel
        }
        
        searchPanel?.center()
        searchPanel?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }
    
    private func openPreferencesWindow() {
        if let win = preferencesWindow, win.isVisible {
            win.makeKeyAndOrderFront(nil)
            return
        }
        
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 480, height: 380),
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = "SurrealFS Preferences"
        window.center()
        let prefView = PreferencesView(viewModel: viewModel)
        window.contentView = NSHostingView(rootView: prefView)
        self.preferencesWindow = window
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }
}
