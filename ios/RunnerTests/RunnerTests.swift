import Flutter
import UIKit
import XCTest
import AVFoundation

class RunnerTests: XCTestCase {

  func testBackgroundAudioModeIsDeclared() {
    let modes = Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String]
    XCTAssertNotNil(modes)
    XCTAssertTrue(modes?.contains("audio") == true)
  }

  func testAudioSessionUsesPlaybackCategory() throws {
    let session = AVAudioSession.sharedInstance()

    try session.setCategory(.playback, mode: .default)
    try session.setActive(true)

    XCTAssertEqual(session.category, .playback)
  }

}
