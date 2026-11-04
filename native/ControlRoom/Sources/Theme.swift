import SwiftUI

enum CRTheme {
  static let canvas = Color(red: 0.035, green: 0.043, blue: 0.055)
  static let panel = Color(red: 0.055, green: 0.065, blue: 0.082)
  static let raised = Color(red: 0.075, green: 0.087, blue: 0.108)
  static let line = Color.white.opacity(0.09)
  static let primary = Color(red: 0.94, green: 0.93, blue: 0.89)
  static let secondary = Color(red: 0.56, green: 0.60, blue: 0.66)
  static let amber = Color(red: 1.0, green: 0.66, blue: 0.20)
  static let cyan = Color(red: 0.18, green: 0.82, blue: 0.92)
  static let violet = Color(red: 0.62, green: 0.45, blue: 1.0)
  static let mint = Color(red: 0.27, green: 0.90, blue: 0.68)
  static let blue = Color(red: 0.30, green: 0.56, blue: 1.0)
  static let coral = Color(red: 1.0, green: 0.40, blue: 0.36)

  static func tint(for kind: NodeKind) -> Color {
    switch kind {
    case .lfo: cyan
    case .sampleHold: mint
    case .stepSequence: blue
    case .slew: violet
    case .clamp: coral
    case .curve: cyan
    case .quantize: amber
    case .math: mint
    case .envelope: coral
    case .macro: amber
    case .scale: violet
    case .sink: amber
    case .externalSource: mint
    case .externalSink: blue
    case .takeover: coral
    }
  }
}

struct GlassPanel: ViewModifier {
  var radius: CGFloat = 16

  func body(content: Content) -> some View {
    content
      .background(.ultraThinMaterial.opacity(0.72))
      .background(CRTheme.panel.opacity(0.78))
      .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
      .overlay(RoundedRectangle(cornerRadius: radius, style: .continuous).stroke(CRTheme.line))
  }
}

extension View {
  func glassPanel(radius: CGFloat = 16) -> some View { modifier(GlassPanel(radius: radius)) }
}

struct GridBackground: View {
  var body: some View {
    Canvas { context, size in
      let minor: CGFloat = 24
      let major: CGFloat = 120
      var path = Path()
      for x in stride(from: 0, through: size.width, by: minor) {
        path.move(to: CGPoint(x: x, y: 0))
        path.addLine(to: CGPoint(x: x, y: size.height))
      }
      for y in stride(from: 0, through: size.height, by: minor) {
        path.move(to: CGPoint(x: 0, y: y))
        path.addLine(to: CGPoint(x: size.width, y: y))
      }
      context.stroke(path, with: .color(.white.opacity(0.022)), lineWidth: 0.5)
      var majorPath = Path()
      for x in stride(from: 0, through: size.width, by: major) {
        majorPath.move(to: CGPoint(x: x, y: 0))
        majorPath.addLine(to: CGPoint(x: x, y: size.height))
      }
      for y in stride(from: 0, through: size.height, by: major) {
        majorPath.move(to: CGPoint(x: 0, y: y))
        majorPath.addLine(to: CGPoint(x: size.width, y: y))
      }
      context.stroke(majorPath, with: .color(.white.opacity(0.043)), lineWidth: 0.7)
    }
    .background(
      RadialGradient(
        colors: [CRTheme.cyan.opacity(0.055), .clear], center: .topLeading, startRadius: 0,
        endRadius: 700)
    )
  }
}
