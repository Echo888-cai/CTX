import AppKit
import SwiftUI

struct PopoverView: View {
    @EnvironmentObject private var model: StatusModel

    private let green = Color(hex: 0x078B45)
    private let kept = Color(hex: 0x7FAEE8)
    private let keptText = Color(hex: 0x2F6FBF)
    private let saveText = Color(hex: 0x2F8F5F)
    private let ink = Color(hex: 0x111311)
    private let muted = Color(hex: 0x6E736F)
    private let quiet = Color(hex: 0x979C98)
    private let line = Color(hex: 0xE8EBE8)
    private let wash = Color(hex: 0xF6F7F6)
    private let paper = Color(hex: 0xFBFCFB)

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            softRule
            overview
            softRule
            toolsList
            softRule
            actions
            softRule
            footer
        }
        .frame(width: 336)
        .background(
            ZStack {
                paper
                LinearGradient(
                    colors: [Color.white.opacity(0.9), wash.opacity(0.55)],
                    startPoint: .top,
                    endPoint: .bottom
                )
            }
        )
        .foregroundStyle(ink)
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(line, lineWidth: 1)
        )
    }

    private var softRule: some View {
        Rectangle()
            .fill(line.opacity(0.9))
            .frame(height: 1)
            .padding(.horizontal, 2)
    }

    private var header: some View {
        HStack(alignment: .center, spacing: 10) {
            Image(nsImage: brandImage)
                .resizable()
                .interpolation(.high)
                .scaledToFit()
                .frame(width: 88, height: 24)
                .accessibilityLabel("CTX")
            Text("让重要的，自然抵达。")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(muted)
                .lineLimit(1)
            Spacer(minLength: 8)
            HStack(spacing: 6) {
                Circle()
                    .fill(model.enabled ? green : quiet)
                    .frame(width: 7, height: 7)
                    .shadow(color: model.enabled ? green.opacity(0.35) : .clear, radius: 3, y: 0)
                Text(model.enabled ? "运行中" : "已暂停")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(model.enabled ? green : muted)
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 5)
            .background(
                Capsule(style: .continuous)
                    .fill(model.enabled ? green.opacity(0.08) : wash)
            )
        }
        .padding(.horizontal, 18)
        .frame(height: 64)
    }

    private var overview: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top) {
                HStack(alignment: .lastTextBaseline, spacing: 2) {
                    Text("\(model.today.reductionPct)")
                        .font(.system(size: 52, weight: .semibold, design: .rounded))
                    Text("%")
                        .font(.system(size: 24, weight: .semibold, design: .rounded))
                }
                .foregroundStyle(green)
                .monospacedDigit()

                Spacer()

                VStack(alignment: .trailing, spacing: 5) {
                    HStack(alignment: .firstTextBaseline, spacing: 5) {
                        Text(model.usdLabel)
                            .font(.system(size: 28, weight: .semibold, design: .rounded))
                            .foregroundStyle(green)
                            .monospacedDigit()
                        if model.avoidedUsdEstimated {
                            estimateBadge
                        }
                    }
                    Text("今日已节省")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(muted)
                }
                .padding(.top, 4)
            }

            Text(model.avoidedLabel)
                .font(.system(size: 28, weight: .semibold, design: .rounded))
                .foregroundStyle(green)
                .monospacedDigit()
                .padding(.top, 28)
            Text("今日已节省 tokens")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(muted)
                .padding(.top, 2)

            composition
                .padding(.top, 16)

            if let error = model.error {
                Text(error)
                    .font(.system(size: 11))
                    .foregroundStyle(Color(hex: 0x8A3B32))
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 10)
            }
        }
        .padding(.horizontal, 18)
        .padding(.top, 22)
        .padding(.bottom, 18)
    }

    private var toolsList: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("工具")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(muted)
                .padding(.bottom, 8)

            if model.tools.isEmpty {
                Text("打开设置接入 Cursor、Claude Code 或 ChatGPT")
                    .font(.system(size: 13))
                    .foregroundStyle(quiet)
                    .frame(minHeight: 36)
            } else {
                ForEach(model.tools) { tool in
                    HStack(spacing: 10) {
                        Circle()
                            .fill(toolDot(tool))
                            .frame(width: 7, height: 7)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 8) {
                                Text(tool.name)
                                    .font(.system(size: 14, weight: .semibold))
                                if !tool.formLabel.isEmpty {
                                    Text(tool.formLabel)
                                        .font(.system(size: 11, weight: .medium))
                                        .foregroundStyle(quiet)
                                }
                            }
                            Text(tool.status)
                                .font(.system(size: 12))
                                .foregroundStyle(tool.installed ? muted : quiet)
                        }
                        Spacer(minLength: 8)
                    }
                    .frame(minHeight: 42)
                }
            }
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
    }

    private func toolDot(_ tool: ToolRow) -> Color {
        if !tool.installed { return quiet }
        if !tool.enabled { return muted }
        return green
    }

    private var composition: some View {
        let rows = model.compositionSegments
        let total = max(1, rows.reduce(0) { $0 + $1.tokens })
        return VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 0) {
                Text("原文 \(model.rawLabel)").font(.system(size: 12)).foregroundStyle(muted)
                Spacer()
                Text("有效输入 \(model.deliveredLabel)").font(.system(size: 12)).foregroundStyle(keptText)
            }
            GeometryReader { geo in
                HStack(spacing: 2) {
                    ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                        RoundedRectangle(cornerRadius: 2, style: .continuous)
                            .fill(segmentColor(row: row, index: index))
                            .frame(width: max(2, geo.size.width * CGFloat(row.tokens) / CGFloat(total)))
                    }
                }
            }
            .frame(height: 9)
            VStack(alignment: .leading, spacing: 7) {
                ForEach(Array(rows.prefix(4).enumerated()), id: \.element.id) { index, row in
                    HStack(spacing: 8) {
                        RoundedRectangle(cornerRadius: 2, style: .continuous)
                            .fill(segmentColor(row: row, index: index))
                            .frame(width: 8, height: 8)
                        Text(row.label).lineLimit(1)
                        Spacer(minLength: 8)
                        Text(fmtCompact(row.tokens))
                            .foregroundStyle(row.kept ? keptText : saveText)
                            .monospacedDigit()
                        Text(String(format: "%.0f%%", Double(row.tokens) * 100 / Double(total)))
                            .foregroundStyle(muted)
                            .monospacedDigit()
                            .frame(width: 34, alignment: .trailing)
                    }
                    .font(.system(size: 12))
                }
            }
        }
        .padding(13)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color.white.opacity(0.72))
                .shadow(color: Color.black.opacity(0.04), radius: 8, y: 2)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(line, lineWidth: 1)
        )
    }

    private var estimateBadge: some View {
        Text("估")
            .font(.system(size: 10, weight: .medium))
            .foregroundStyle(muted)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(RoundedRectangle(cornerRadius: 4, style: .continuous).fill(Color.white))
    }

    private func segmentColor(row: CompositionRow, index: Int) -> Color {
        if row.kept {
            return kept
        }
        let palette: [UInt32] = [0x4FA97B, 0x6CBB92, 0x86CFA8, 0xA3DCBE, 0xBFE8D2, 0xD8F1E3, 0xEAF7EF]
        return Color(hex: palette[max(0, index - 1) % palette.count])
    }

    private var actions: some View {
        VStack(spacing: 10) {
            Button {
                model.openDashboard()
            } label: {
                Text(model.launching ? "正在打开…" : "打开仪表盘")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(CTXButtonStyle(emphasis: true))

            Button {
                model.setEnabled(!model.enabled)
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: model.enabled ? "pause.fill" : "play.fill")
                    Text(model.enabled ? "暂停" : "继续")
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(CTXButtonStyle(emphasis: false))
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
    }

    private var footer: some View {
        HStack {
            Toggle("登录时启动", isOn: Binding(
                get: { model.loginEnabled },
                set: { model.toggleLogin($0) }
            ))
            .toggleStyle(.checkbox)
            .font(.system(size: 12, weight: .medium))

            Spacer()

            Button("退出 CTX") { model.quit() }
                .buttonStyle(.plain)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(muted)
        }
        .padding(.horizontal, 18)
        .frame(height: 54)
    }

    private var brandImage: NSImage {
        if let previewPath = ProcessInfo.processInfo.environment["CTX_WORDMARK"],
           let image = NSImage(contentsOfFile: previewPath) {
            return image
        }
        for name in ["ctx-wordmark", "ctx-menubar"] {
            if let url = Bundle.main.url(forResource: name, withExtension: "png"),
               let image = NSImage(contentsOf: url) {
                return image
            }
        }
        return NSImage(size: NSSize(width: 760, height: 220))
    }
}

private struct CTXButtonStyle: ButtonStyle {
    var emphasis: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 14, weight: .semibold))
            .frame(height: 48)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(emphasis ? Color.black : Color.white.opacity(0.88))
                    .shadow(
                        color: emphasis ? Color.black.opacity(0.18) : Color.black.opacity(0.04),
                        radius: emphasis ? 10 : 6,
                        y: emphasis ? 3 : 1
                    )
            )
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(emphasis ? Color.black : Color(hex: 0xE0E3E0), lineWidth: 1)
            )
            .foregroundStyle(emphasis ? Color.white : Color(hex: 0x111311))
            .opacity(configuration.isPressed ? 0.78 : 1)
            .scaleEffect(configuration.isPressed ? 0.985 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

private extension Color {
    init(hex: UInt32, alpha: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: alpha
        )
    }
}
