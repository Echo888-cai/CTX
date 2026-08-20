import SwiftUI
import WebKit

/// Main app surface: the local CTX dashboard inside a real window, like CC Switch.
struct DashboardView: View {
    @ObservedObject private var loader = DashboardLoader.shared

    var body: some View {
        ZStack {
            DashboardWebView(url: loader.url, reloadToken: loader.reloadToken)
            if let message = loader.message {
                VStack(spacing: 12) {
                    ProgressView()
                    Text(message)
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                }
                .padding(24)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .frame(minWidth: 880, minHeight: 620)
        .onAppear { loader.start() }
    }
}

@MainActor
final class DashboardLoader: ObservableObject {
    static let shared = DashboardLoader()

    let port: UInt16 = 8741
    @Published var message: String? = "正在打开 CTX…"
    @Published var reloadToken = 0
    private var child: Process?
    private var attempts = 0

    var url: URL { URL(string: "http://127.0.0.1:\(port)/")! }

    func start() {
        attempts = 0
        message = "正在打开 CTX…"
        spawnServer()
        poll()
    }

    private func spawnServer() {
        guard let bin = ctxURL() else {
            message = "找不到 ctx 命令。先安装 CLI。"
            return
        }
        if child?.isRunning == true { return }
        let proc = Process()
        proc.executableURL = bin
        proc.arguments = ["app", "--port", "\(port)", "--no-open"]
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        do {
            try proc.run()
            child = proc
        } catch {
            message = error.localizedDescription
        }
    }

    private func poll() {
        attempts += 1
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let up = self?.isUp() ?? false
            DispatchQueue.main.async {
                guard let self else { return }
                if up {
                    self.message = nil
                    self.reloadToken += 1
                    return
                }
                if self.attempts >= 40 {
                    self.message = "仪表盘没有起来。终端运行：ctx app"
                    return
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
                    self.poll()
                }
            }
        }
    }

    private func isUp() -> Bool {
        guard let probe = URL(string: "http://127.0.0.1:\(port)/api/health") else { return false }
        var request = URLRequest(url: probe)
        request.timeoutInterval = 0.35
        var up = false
        let sem = DispatchSemaphore(value: 0)
        URLSession.shared.dataTask(with: request) { _, response, _ in
            if let http = response as? HTTPURLResponse, (200 ..< 500).contains(http.statusCode) {
                up = true
            }
            sem.signal()
        }.resume()
        _ = sem.wait(timeout: .now() + 0.4)
        return up
    }
}

struct DashboardWebView: NSViewRepresentable {
    let url: URL
    let reloadToken: Int

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    final class Coordinator {
        var lastToken = -1
    }

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        let view = WKWebView(frame: .zero, configuration: config)
        view.setValue(false, forKey: "drawsBackground")
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        if context.coordinator.lastToken != reloadToken {
            context.coordinator.lastToken = reloadToken
            view.load(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 8))
        }
    }
}
