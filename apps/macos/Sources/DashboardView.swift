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
    private var stopping = false

    var url: URL { URL(string: "http://127.0.0.1:\(port)/")! }

    func start() {
        stopping = false
        attempts = 0
        message = "正在接入已安装的工具…"
        spawnServer()
        poll()
    }

    func stop() {
        stopping = true
        let port = self.port
        let proc = child
        child = nil
        let group = DispatchGroup()
        group.enter()
        DispatchQueue.global(qos: .userInitiated).async {
            Self.post("/api/pause", port: port)
            _ = try? CtxCLI.run(["lifecycle", "deactivate"])
            group.leave()
        }
        _ = group.wait(timeout: .now() + 3)
        if let proc, proc.isRunning {
            proc.terminate()
            proc.waitUntilExit()
        }
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
        var env = ProcessInfo.processInfo.environment
        env["CTX_APP_BUNDLE"] = Bundle.main.bundlePath
        proc.environment = env
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        do {
            try proc.run()
            child = proc
        } catch {
            message = error.localizedDescription
        }
    }

    private static func post(_ path: String, port: UInt16) {
        guard let url = URL(string: "http://127.0.0.1:\(port)\(path)") else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = 2
        let sem = DispatchSemaphore(value: 0)
        URLSession.shared.dataTask(with: request) { _, _, _ in sem.signal() }.resume()
        _ = sem.wait(timeout: .now() + 2)
    }

    private func poll() {
        attempts += 1
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let up = self?.isUp() ?? false
            DispatchQueue.main.async {
                guard let self else { return }
                if self.stopping {
                    return
                }
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
