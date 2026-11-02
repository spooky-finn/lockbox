// Keychain receives only an encrypted envelope. Decrypting requires an actual
// Secure Enclave key agreement with the current fingerprint set, not a UI check.
import CryptoKit
import Foundation
import LocalAuthentication
import Security

private let domain = Data("lockbox.touch-id.v3".utf8)

private struct Envelope: Codable {
    let version: Int
    let enclaveKey: Data
    let ephemeralPublicKey: Data
    let ciphertext: Data
}

private enum StorageError: LocalizedError {
    case invalidEnvelope
    case unavailable

    var errorDescription: String? {
        switch self {
        case .invalidEnvelope: return "Invalid Touch ID storage — enter the master password"
        case .unavailable: return "Secure Enclave is unavailable — use the master password"
        }
    }
}

private func result(
    _ errorOut: UnsafeMutablePointer<Unmanaged<CFError>?>,
    _ operation: () throws -> Data
) -> Unmanaged<CFMutableData>? {
    do {
        var data = try operation()
        defer { data.resetBytes(in: 0..<data.count) }
        // Return an owned mutable buffer so Rust can wipe decrypted bytes after
        // copying them into its Zeroizing allocation.
        return Unmanaged.passRetained(CFDataCreateMutableCopy(nil, 0, data as CFData)!)
    } catch {
        let error = error as NSError
        errorOut.pointee = Unmanaged.passRetained(CFErrorCreate(
            nil, error.domain as CFString, error.code, error.userInfo as CFDictionary
        )!)
        return nil
    }
}

@_cdecl("lockbox_touch_id_enclave_available")
func enclaveAvailable() -> Bool {
    SecureEnclave.isAvailable
}

@_cdecl("lockbox_touch_id_seal")
func seal(
    _ input: CFData,
    _ account: CFData,
    _ errorOut: UnsafeMutablePointer<Unmanaged<CFError>?>
) -> Unmanaged<CFMutableData>? {
    result(errorOut) {
        guard SecureEnclave.isAvailable else { throw StorageError.unavailable }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            [.privateKeyUsage, .biometryCurrentSet], &error
        ) else {
            throw error!.takeRetainedValue()
        }
        // CryptoKit does not add a permanent Keychain item. dataRepresentation
        // is an opaque, device-bound blob; it never contains an exportable key.
        let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(accessControl: access)
        let ephemeral = P256.KeyAgreement.PrivateKey()
        // Encryption uses only the enclave PUBLIC key, so password refresh does
        // not need a fingerprint. Only decryption uses its protected private key.
        let secret = try ephemeral.sharedSecretFromKeyAgreement(with: enclave.publicKey)
        let key = secret.hkdfDerivedSymmetricKey(
            using: SHA256.self, salt: domain, sharedInfo: account as Data, outputByteCount: 32
        )
        let box = try AES.GCM.seal(input as Data, using: key, authenticating: account as Data)
        guard let ciphertext = box.combined else { throw StorageError.invalidEnvelope }
        return try JSONEncoder().encode(Envelope(
            version: 1, enclaveKey: enclave.dataRepresentation,
            ephemeralPublicKey: ephemeral.publicKey.x963Representation, ciphertext: ciphertext
        ))
    }
}

@_cdecl("lockbox_touch_id_open")
func open(
    _ input: CFData,
    _ account: CFData,
    _ contextPointer: UnsafeRawPointer,
    _ errorOut: UnsafeMutablePointer<Unmanaged<CFError>?>
) -> Unmanaged<CFMutableData>? {
    result(errorOut) {
        guard CFDataGetLength(input) <= 65536 else { throw StorageError.invalidEnvelope }
        let envelope = try JSONDecoder().decode(Envelope.self, from: input as Data)
        guard envelope.version == 1 else { throw StorageError.invalidEnvelope }
        let context = Unmanaged<LAContext>.fromOpaque(contextPointer).takeUnretainedValue()
        defer { context.invalidate() }
        let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(
            dataRepresentation: envelope.enclaveKey, authenticationContext: context
        )
        let ephemeral = try P256.KeyAgreement.PublicKey(x963Representation: envelope.ephemeralPublicKey)
        // The Secure Enclave enforces biometryCurrentSet here. A copied envelope,
        // previous evaluatePolicy success or cancelled prompt cannot release data.
        let secret = try enclave.sharedSecretFromKeyAgreement(with: ephemeral)
        let key = secret.hkdfDerivedSymmetricKey(
            using: SHA256.self, salt: domain, sharedInfo: account as Data, outputByteCount: 32
        )
        return try AES.GCM.open(
            AES.GCM.SealedBox(combined: envelope.ciphertext),
            using: key, authenticating: account as Data
        )
    }
}
