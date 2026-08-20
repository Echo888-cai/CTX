import AppKit
import Foundation
import SwiftUI

@MainActor
final class StatusModel: ObservableObject {
    @Published var enabled = true
    @Published var version = ""
    @Published var today = Totals()
    @Published var week = Totals()
    @Published var pages = 0
    @Published var storeBytes: Int64 = 0
    @Published var harnesses: [HarnessRow] = []
    @Published var tools: [ToolRow] = []
    @Published var models: [ModelSaveRow] = []
    @Published var avoidedUsd: Double?
    @Published var avoidedUsdEstimated = false
    @Published var composition: [CompositionRow] = []
    @Published var unpricedModels = 0
    @Published var error: String?
    @Published var launching = false
    @Published var loginEnabled = LoginItem.isEnabled

    var menuLabel: String {
        if ctxURL() == nil {
            return "CTX"
        }
        if !enabled {
            return "暂停"
        }
        if today.raw == 0 {
            return "CTX"
        }
        return "↓\(today.reductionPct)%"
    }

    var avoidedLabel: String { fmtCompact(today.avoided) }
    var rawLabel: String { fmtCompact(today.raw) }
    var deliveredLabel: String { fmtCompact(today.delivered) }
    var usdLabel: String {
        avoidedUsd == nil ? "未定价" : fmtUsd(avoidedUsd)
    }

    /// Segments that add up to today's raw context: what reached the model
    /// first, then why the rest did not.
    var compositionSegments: [CompositionRow] {
        let rows = composition.filter { $0.tokens > 0 }
        if !rows.isEmpty {
            return rows
        }
        guard today.raw > 0 else { return [] }
        return [
            CompositionRow(key: "delivered", label: "有效输入", tokens: today.delivered, kept: true),
            CompositionRow(key: "avoided", label: "已节省", tokens: today.avoided, kept: false),
        ].filter { $0.tokens > 0 }
    }

    private var timer: Timer?

    init() {
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in
                self?.refresh()
            }
        }
        if let timer {
            RunLoop.main.add(timer, forMode: .common)
        }
    }

    func refresh() {
        loginEnabled = LoginItem.isEnabled
        do {
            let data = try CtxCLI.run(["status", "--json"])
            let payload = try JSONDecoder().decode(StatusPayload.self, from: data)
            enabled = payload.enabled
            version = payload.version
            today = payload.today
            week = payload.week
            pages = payload.pages
            storeBytes = payload.storeBytes
            harnesses = payload.byHarness
            tools = payload.tools
            models = payload.models
            avoidedUsd = payload.avoidedUsd
            avoidedUsdEstimated = payload.avoidedUsdEstimated
            composition = payload.composition
            unpricedModels = payload.unpricedModels
            error = nil
            NotificationCenter.default.post(name: .ctxStatusDidChange, object: nil)
        } catch {
            self.error = error.localizedDescription
            NotificationCenter.default.post(name: .ctxStatusDidChange, object: nil)
        }
    }

    func setEnabled(_ on: Bool) {
        do {
            _ = try CtxCLI.run([on ? "resume" : "pause"])
            enabled = on
            refresh()
            NotificationCenter.default.post(name: .ctxStatusDidChange, object: nil)
        } catch {
            self.error = error.localizedDescription
        }
    }

    func openDashboard() {
        launching = true
        DispatchQueue.global(qos: .userInitiated).async {
            if let port = ProcessInfo.processInfo.environment["CTX_PORT"], !port.isEmpty {
                _ = try? CtxCLI.run(["app", "--port", port])
            } else {
                _ = try? CtxCLI.run(["app"])
            }
            DispatchQueue.main.async {
                self.launching = false
            }
        }
    }

    func toggleLogin(_ on: Bool) {
        do {
            try LoginItem.setEnabled(on)
            loginEnabled = LoginItem.isEnabled
        } catch {
            self.error = error.localizedDescription
        }
    }

    func quit() {
        NSApplication.shared.terminate(nil)
    }
}

struct Totals: Decodable {
    var raw = 0
    var delivered = 0
    var avoided = 0
    var reductionPct = 0

    enum CodingKeys: String, CodingKey {
        case raw, delivered, avoided
        case reductionPct = "reduction_pct"
    }
}

struct ModelSaveRow: Decodable, Identifiable {
    var id: String
    var name: String
    var avoided: Int
    var avoidedUsd: Double?
    var reductionPct: Double
    var inputUsdPerMtok: Double?
    var priceEstimate = false

    /// `$2/M`, or empty when the model has no rate.
    var rateLabel: String {
        guard let rate = inputUsdPerMtok, rate > 0 else { return "" }
        return rate == rate.rounded()
            ? String(format: "$%.0f/M", rate)
            : String(format: "$%.2f/M", rate)
    }

    enum CodingKeys: String, CodingKey {
        case id, name, avoided
        case avoidedUsd = "avoided_usd"
        case reductionPct = "reduction_pct"
        case inputUsdPerMtok = "input_usd_per_mtok"
        case priceEstimate = "price_estimate"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        avoided = try c.decode(Int.self, forKey: .avoided)
        avoidedUsd = try? c.decode(Double.self, forKey: .avoidedUsd)
        reductionPct = (try? c.decode(Double.self, forKey: .reductionPct)) ?? 0
        inputUsdPerMtok = try? c.decode(Double.self, forKey: .inputUsdPerMtok)
        priceEstimate = (try? c.decode(Bool.self, forKey: .priceEstimate)) ?? false
    }
}

struct CompositionRow: Decodable, Identifiable {
    var key: String
    var label: String
    var tokens: Int
    var kept: Bool

    var id: String { key }
}

struct HarnessRow: Decodable, Identifiable {
    var name: String
    var raw: Int
    var delivered: Int
    var avoided: Int
    var reductionPct: Int

    var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name, raw, delivered, avoided
        case reductionPct = "reduction_pct"
    }
}

struct ToolRow: Decodable, Identifiable {
    var id: String
    var name: String
    var installed: Bool
    var enabled: Bool
    var capability: String
    var status: String
    var formLabel: String

    enum CodingKeys: String, CodingKey {
        case id, name, installed, enabled, capability, status
        case formLabel = "form_label"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        installed = (try? c.decode(Bool.self, forKey: .installed)) ?? false
        enabled = (try? c.decode(Bool.self, forKey: .enabled)) ?? true
        capability = (try? c.decode(String.self, forKey: .capability)) ?? ""
        status = (try? c.decode(String.self, forKey: .status)) ?? (installed ? "" : "未安装")
        formLabel = (try? c.decode(String.self, forKey: .formLabel)) ?? ""
    }
}

private struct StatusPayload: Decodable {
    var ok: Bool
    var enabled: Bool
    var version: String
    var today: Totals
    var week: Totals
    var pages: Int
    var storeBytes: Int64
    var byHarness: [HarnessRow]
    var tools: [ToolRow]
    var models: [ModelSaveRow]
    var avoidedUsd: Double?
    var avoidedUsdEstimated: Bool
    var composition: [CompositionRow]
    var unpricedModels: Int

    enum CodingKeys: String, CodingKey {
        case ok, enabled, version, today, week, pages, models, composition
        case storeBytes = "store_bytes"
        case byHarness = "by_harness"
        case tools
        case avoidedUsd = "avoided_usd"
        case avoidedUsdEstimated = "avoided_usd_estimated"
        case unpricedModels = "unpriced_models"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        ok = try c.decode(Bool.self, forKey: .ok)
        enabled = try c.decode(Bool.self, forKey: .enabled)
        version = try c.decode(String.self, forKey: .version)
        today = try c.decode(Totals.self, forKey: .today)
        week = try c.decode(Totals.self, forKey: .week)
        pages = try c.decode(Int.self, forKey: .pages)
        storeBytes = try c.decode(Int64.self, forKey: .storeBytes)
        byHarness = try c.decodeIfPresent([HarnessRow].self, forKey: .byHarness) ?? []
        tools = try c.decodeIfPresent([ToolRow].self, forKey: .tools) ?? []
        models = try c.decodeIfPresent([ModelSaveRow].self, forKey: .models) ?? []
        avoidedUsd = (try? c.decode(Double.self, forKey: .avoidedUsd))
        avoidedUsdEstimated = try c.decodeIfPresent(Bool.self, forKey: .avoidedUsdEstimated) ?? false
        composition = try c.decodeIfPresent([CompositionRow].self, forKey: .composition) ?? []
        unpricedModels = try c.decodeIfPresent(Int.self, forKey: .unpricedModels) ?? 0
    }
}

enum CtxCLI {
    static func run(_ args: [String]) throws -> Data {
        guard let bin = ctxURL() else {
            throw CtxError.missingBinary
        }
        let proc = Process()
        proc.executableURL = bin
        proc.arguments = args
        let out = Pipe()
        let err = Pipe()
        proc.standardOutput = out
        proc.standardError = err
        try proc.run()
        proc.waitUntilExit()
        let data = out.fileHandleForReading.readDataToEndOfFile()
        if proc.terminationStatus != 0 {
            let message = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)
                ?? "ctx exited \(proc.terminationStatus)"
            throw CtxError.command(message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return data
    }
}

enum CtxError: LocalizedError {
    case missingBinary
    case command(String)

    var errorDescription: String? {
        switch self {
        case .missingBinary:
            return "找不到 ctx。先安装 CLI：curl -fsSL https://raw.githubusercontent.com/Echo888-cai/CTX/main/install.sh | bash"
        case .command(let message):
            return message.isEmpty ? "ctx 命令失败" : message
        }
    }
}

func ctxURL() -> URL? {
    if let override = ProcessInfo.processInfo.environment["CTX_BIN"] {
        let url = URL(fileURLWithPath: override)
        if FileManager.default.isExecutableFile(atPath: url.path) {
            return url
        }
    }
    let home = FileManager.default.homeDirectoryForCurrentUser
    let candidates = [
        home.appendingPathComponent(".cargo/bin/ctx"),
        URL(fileURLWithPath: "/opt/homebrew/bin/ctx"),
        URL(fileURLWithPath: "/usr/local/bin/ctx"),
    ]
    return candidates.first { FileManager.default.isExecutableFile(atPath: $0.path) }
}

func fmtCompact(_ n: Int) -> String {
    if n >= 1_000_000 {
        return String(format: "%.1fM", Double(n) / 1_000_000)
    }
    if n >= 10_000 {
        return String(format: "%.1fK", Double(n) / 1_000)
    }
    if n >= 1_000 {
        return String(format: "%.2fK", Double(n) / 1_000)
    }
    return "\(n)"
}

func fmtUsd(_ n: Double?) -> String {
    guard let n else { return "—" }
    if n >= 0.01 { return String(format: "$%.2f", n) }
    if n > 0 { return "<$0.01" }
    return "$0.00"
}

enum LoginItem {
    static let label = "ai.ctx.bar"

    static var plistURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents/\(label).plist")
    }

    static var isEnabled: Bool {
        FileManager.default.fileExists(atPath: plistURL.path)
    }

    static func setEnabled(_ on: Bool) throws {
        let plist = plistURL
        if on {
            let app = Bundle.main.bundleURL.path
            let binary = Bundle.main.executableURL?.path ?? "\(app)/Contents/MacOS/CTX"
            let body = """
            <?xml version="1.0" encoding="UTF-8"?>
            <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
            <plist version="1.0">
            <dict>
              <key>Label</key>
              <string>\(label)</string>
              <key>ProgramArguments</key>
              <array>
                <string>\(binary)</string>
              </array>
              <key>RunAtLoad</key>
              <true/>
              <key>KeepAlive</key>
              <false/>
              <key>LimitLoadToSessionType</key>
              <string>Aqua</string>
            </dict>
            </plist>
            """
            try FileManager.default.createDirectory(
                at: plist.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try body.write(to: plist, atomically: true, encoding: .utf8)
            // Never unload/bootout while we are running — that kills this process.
        } else {
            // Only remove the login item. Do not bootout: this process may be
            // the LaunchAgent itself.
            if FileManager.default.fileExists(atPath: plist.path) {
                try FileManager.default.removeItem(at: plist)
            }
        }
    }

    private static func shell(_ args: [String]) throws -> Int32 {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        proc.arguments = args
        try proc.run()
        proc.waitUntilExit()
        return proc.terminationStatus
    }
}
