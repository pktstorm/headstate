// The iOS side of tauri-plugin-headstate-keys.
//
// Holds the step-up signing keys in the Secure Enclave and keeps the
// session identity (made in Rust) in the Keychain. The Rust side
// (src/lib.rs, src/wire.rs) documents the commands and the JSON they
// exchange; this file matches it. Every byte string is standard base64.
//
// # Where the keys live
//
// A Secure Enclave key is never in this process. CryptoKit hands back a
// `dataRepresentation`: an opaque, enclave-wrapped blob that only this
// device's enclave can turn back into a usable key. That blob is what is
// persisted, as a Keychain generic-password item, which is Apple's own
// guidance for SE keys. Deleting the item is the only handle we have on
// the key, so `destroy` deletes the items and the keys are gone.
//
// # Access control: `.userPresence`, not `.biometryCurrentSet`
//
// The step-up keys are created with `SecAccessControl` flags
// `[.privateKeyUsage, .userPresence]`. `.privateKeyUsage` is what makes
// the enclave enforce the policy on signing; `.userPresence` means Face
// ID or Touch ID with the device passcode as fallback, which is the
// "biometric or device passcode" the spec asks for and what a phone
// without biometrics set up needs at all.
//
// The alternative, `.biometryCurrentSet`, makes the key unusable the
// moment the biometric enrolment changes. That is stronger against an
// attacker who knows the passcode and enrols their own face, but it also
// refuses the passcode fallback, invalidates the pairing whenever the
// user re-enrols a finger, and excludes devices with no biometrics.
// Since a pairing can be revoked from the desktop at any time, the
// spec's choice stands; revisit if the threat model changes.
//
// The items themselves are `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`:
// readable only while the phone is unlocked, never synced to iCloud
// Keychain, never restored onto another device from a backup. The
// session identity gets the same protection; it is the one key whose
// bytes do leave the Keychain (into rustls), so it is not in the
// enclave -- src/lib.rs explains why.
//
// # One prompt for two keys
//
// Both signing keys are loaded with the same `LAContext`. The first
// signature evaluates the access control and shows the sheet; the
// enclave then honours that context for the second key without asking
// again. If a device turns out to prompt twice, the pairing walkthrough
// will catch it and the fix is in this file only.
//
// # Threading
//
// Tauri calls plugin commands on a serial background queue. CryptoKit
// and Security are fine with that, and the authentication sheet is
// presented by the system, so nothing here touches the main queue.

import CryptoKit
import Foundation
import LocalAuthentication
import Security
import Tauri
import UIKit

struct SignArgs: Decodable {
  let message: String
  let reason: String
}

struct StoreSessionArgs: Decodable {
  let certDer: String
  let keyPkcs8: String
}

struct PublicKeysResponse: Encodable {
  let ecdsaP256: String
  let mldsa65: String?
}

struct SignaturesResponse: Encodable {
  let ecdsa: String
  let mldsa: String?
}

struct SessionResponse: Encodable {
  let certDer: String
  let keyPkcs8: String
}

/// Rejection codes the Rust side maps to `Error` variants.
enum Code {
  static let notGenerated = "notGenerated"
  static let cancelled = "cancelled"
  static let authFailed = "authFailed"
  static let unavailable = "unavailable"
  static let malformed = "malformed"
}

enum KeychainError: Error {
  case status(OSStatus)
}

/// The Keychain items, one service, one account per thing kept.
enum Item: String, CaseIterable {
  case stepUpEcdsa = "stepup-ecdsa-p256"
  case stepUpMldsa = "stepup-mldsa-65"
  case sessionCert = "session-cert-der"
  case sessionKey = "session-key-pkcs8"

  static let service = "com.pktstorm.headstate.companion.keys"

  var query: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: Item.service,
      kSecAttrAccount as String: rawValue,
    ]
  }

  func read() throws -> Data? {
    var query = self.query
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var out: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &out)
    switch status {
    case errSecSuccess:
      return out as? Data
    case errSecItemNotFound:
      return nil
    default:
      throw KeychainError.status(status)
    }
  }

  func write(_ data: Data) throws {
    try delete()
    var attrs = query
    attrs[kSecValueData as String] = data
    attrs[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
    let status = SecItemAdd(attrs as CFDictionary, nil)
    guard status == errSecSuccess else { throw KeychainError.status(status) }
  }

  func delete() throws {
    let status = SecItemDelete(query as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else {
      throw KeychainError.status(status)
    }
  }
}

class HeadstateKeysPlugin: Plugin {

  // MARK: Commands

  @objc public func generate(_ invoke: Invoke) throws {
    guard SecureEnclave.isAvailable else {
      invoke.reject("this device has no Secure Enclave", code: Code.unavailable)
      return
    }
    // Replace, never add: a half-generated set from a failed earlier
    // attempt must not survive next to a new one.
    try destroyAll()

    let access = try accessControl()
    let ecdsa = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
    try Item.stepUpEcdsa.write(ecdsa.dataRepresentation)

    var mldsaPublic: String? = nil
    if #available(iOS 26, *) {
      // A throw here is "this enclave cannot hold an ML-DSA key", not
      // an error: the phone pairs with ECDSA alone and the desktop
      // records that. See "Post-quantum posture" in the design spec.
      do {
        let mldsa = try SecureEnclave.MLDSA65.PrivateKey(accessControl: access)
        try Item.stepUpMldsa.write(mldsa.dataRepresentation)
        mldsaPublic = mldsa.publicKey.rawRepresentation.base64EncodedString()
      } catch {
        Logger.info("headstate-keys: no ML-DSA-65 key on this device: \(error)")
      }
    }

    invoke.resolve(
      PublicKeysResponse(
        ecdsaP256: ecdsa.publicKey.x963Representation.base64EncodedString(),
        mldsa65: mldsaPublic))
  }

  @objc public func publicKeys(_ invoke: Invoke) throws {
    guard let ecdsaBlob = try Item.stepUpEcdsa.read() else {
      invoke.reject("no device keys", code: Code.notGenerated)
      return
    }
    // Public keys need no authentication, so no context is passed.
    let ecdsa = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: ecdsaBlob)
    var mldsaPublic: String? = nil
    if #available(iOS 26, *), let mldsaBlob = try Item.stepUpMldsa.read() {
      let mldsa = try SecureEnclave.MLDSA65.PrivateKey(dataRepresentation: mldsaBlob)
      mldsaPublic = mldsa.publicKey.rawRepresentation.base64EncodedString()
    }
    invoke.resolve(
      PublicKeysResponse(
        ecdsaP256: ecdsa.publicKey.x963Representation.base64EncodedString(),
        mldsa65: mldsaPublic))
  }

  @objc public func sign(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SignArgs.self)
    guard let message = Data(base64Encoded: args.message) else {
      invoke.reject("message is not base64", code: Code.malformed)
      return
    }
    guard let ecdsaBlob = try Item.stepUpEcdsa.read() else {
      invoke.reject("no device keys", code: Code.notGenerated)
      return
    }

    let context = LAContext()
    context.localizedReason = args.reason

    do {
      let ecdsa = try SecureEnclave.P256.Signing.PrivateKey(
        dataRepresentation: ecdsaBlob, authenticationContext: context)
      // `signature(for:)` hashes with SHA256; `rawRepresentation` is
      // r || s, 64 bytes, exactly what the desktop verifies.
      let ecdsaSig = try ecdsa.signature(for: message).rawRepresentation

      var mldsaSig: String? = nil
      if #available(iOS 26, *), let mldsaBlob = try Item.stepUpMldsa.read() {
        let mldsa = try SecureEnclave.MLDSA65.PrivateKey(
          dataRepresentation: mldsaBlob, authenticationContext: context)
        // No `context:` argument: pure ML-DSA with the empty context
        // string, as the desktop's verifier expects.
        mldsaSig = try mldsa.signature(for: message).base64EncodedString()
      }

      invoke.resolve(
        SignaturesResponse(ecdsa: ecdsaSig.base64EncodedString(), mldsa: mldsaSig))
    } catch {
      let (code, message) = classify(error)
      invoke.reject(message, code: code)
    }
  }

  @objc public func destroy(_ invoke: Invoke) throws {
    try destroyAll()
    invoke.resolve()
  }

  @objc public func storeSession(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(StoreSessionArgs.self)
    guard let cert = Data(base64Encoded: args.certDer),
      let key = Data(base64Encoded: args.keyPkcs8)
    else {
      invoke.reject("session identity is not base64", code: Code.malformed)
      return
    }
    try Item.sessionCert.write(cert)
    try Item.sessionKey.write(key)
    invoke.resolve()
  }

  @objc public func loadSession(_ invoke: Invoke) throws {
    guard let cert = try Item.sessionCert.read(), let key = try Item.sessionKey.read() else {
      invoke.reject("no session identity", code: Code.notGenerated)
      return
    }
    invoke.resolve(
      SessionResponse(certDer: cert.base64EncodedString(), keyPkcs8: key.base64EncodedString()))
  }

  // MARK: Helpers

  private func destroyAll() throws {
    for item in Item.allCases {
      try item.delete()
    }
  }

  private func accessControl() throws -> SecAccessControl {
    var error: Unmanaged<CFError>?
    guard
      let access = SecAccessControlCreateWithFlags(
        kCFAllocatorDefault,
        kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        [.privateKeyUsage, .userPresence],
        &error)
    else {
      throw error!.takeRetainedValue() as Error
    }
    return access
  }

  /// A dismissed sheet is `cancelled`; anything else the enclave refused
  /// is `authFailed`; the rest is a plain rejection with no code.
  private func classify(_ error: Error) -> (String?, String) {
    let ns = error as NSError
    if ns.domain == LAErrorDomain {
      let cancelled: [LAError.Code] = [.userCancel, .appCancel, .systemCancel, .userFallback]
      if let code = LAError.Code(rawValue: ns.code), cancelled.contains(code) {
        return (Code.cancelled, "the confirmation prompt was cancelled")
      }
      return (Code.authFailed, ns.localizedDescription)
    }
    if let ck = error as? CryptoKitError, case .authenticationFailure = ck {
      return (Code.authFailed, "the Secure Enclave refused the signature")
    }
    return (nil, "\(error)")
  }
}

@_cdecl("init_plugin_headstate_keys")
func initPlugin() -> Plugin {
  return HeadstateKeysPlugin()
}
