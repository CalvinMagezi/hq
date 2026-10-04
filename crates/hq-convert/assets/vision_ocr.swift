import Vision
import AppKit

guard CommandLine.arguments.count > 1 else {
    FileHandle.standardError.write("usage: vision_ocr.swift <image-path>\n".data(using: .utf8)!)
    exit(1)
}

guard let img = NSImage(contentsOfFile: CommandLine.arguments[1]),
      let cgImage = img.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    FileHandle.standardError.write("could not load image: \(CommandLine.arguments[1])\n".data(using: .utf8)!)
    exit(1)
}

let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true

do {
    let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
    try handler.perform([request])
} catch {
    FileHandle.standardError.write("Vision request failed: \(error)\n".data(using: .utf8)!)
    exit(1)
}

for observation in request.results ?? [] {
    if let candidate = observation.topCandidates(1).first {
        print(candidate.string)
    }
}
