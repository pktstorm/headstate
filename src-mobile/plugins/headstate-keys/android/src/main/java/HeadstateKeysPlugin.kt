// The Android side of tauri-plugin-headstate-keys.
//
// Holds the step-up signing keys in the Android Keystore and keeps the
// session identity (made in Rust) in an app-private file wrapped by a
// Keystore AES key. The Rust side (src/lib.rs, src/wire.rs) documents
// the commands and the JSON they exchange; this file matches it. Every
// byte string is standard base64.
//
// # Access control
//
// Both signing keys are `setUserAuthenticationRequired(true)` with
// `AUTH_BIOMETRIC_STRONG or AUTH_DEVICE_CREDENTIAL` (API 30+), the
// "biometric or device passcode" of the spec, and
// `setInvalidatedByBiometricEnrollment(false)` so re-enrolling a finger
// does not silently unpair the phone; the same trade-off as `.userPresence`
// over `.biometryCurrentSet` on iOS, recorded in the Swift file. Below
// API 30 a key cannot be gated on the credential, so those devices get
// a biometric-only key and a biometric-only prompt.
//
// # One prompt for two keys
//
// A `BiometricPrompt.CryptoObject` binds one authentication to ONE
// `Signature` operation, and the ECDSA key is authorised that way:
// `setUserAuthenticationParameters(0, ...)`, prompt with the initialised
// `Signature` as the crypto object, sign. The ML-DSA key cannot ride on
// the same object, so it is time-bound instead: a ten-second window
// after any successful authentication of the allowed kinds. In
// practice the ECDSA prompt opens that window and the ML-DSA signature
// follows without a second sheet. If the window does not cover it
// (`UserNotAuthenticatedException`), the plugin shows one more prompt
// without a crypto object and signs inside the fresh window; that path
// exists so a device where the bound authentication does not count
// for time-bound keys still works, at the cost of the second prompt
// this design is trying to avoid. Confirm on a real Android 17 device
// in the pairing walkthrough.
//
// # ML-DSA-65 is optional
//
// Made only when `Build.VERSION.SDK_INT >= 37` (API 37, Android 17)
// and `FEATURE_HARDWARE_KEYSTORE >= 500` (KeyMint 5, the documented
// requirement); any exception from generation, and a key that reports
// a security level below TEE, both mean "no ML-DSA" and the phone pairs
// with ECDSA alone. The algorithm names are the documented values of
// `KeyProperties.KEY_ALGORITHM_ML_DSA_65` ("ML-DSA-65") and the
// `Signature` family name ("ML-DSA"), spelled out so this compiles
// against SDK 36.
//
// # The session identity
//
// A Keystore key cannot be exported, and rustls needs the bytes, so the
// session key is generated in Rust and only STORED here: the PKCS#8 and
// certificate DER, as a small JSON document, encrypted with 256-bit AES-GCM
// under a non-exportable Keystore key and written to SharedPreferences.
// This is the construction androidx's EncryptedSharedPreferences used
// before it was deprecated, done by hand to avoid the dependency.
//
// # Threading
//
// Tauri calls commands off the main thread. BiometricPrompt must be
// created and shown on it, so `sign` hops over with `runOnUiThread` and
// resolves the invoke from the prompt's callbacks.

package com.pktstorm.headstate.keys

import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.UserNotAuthenticatedException
import android.util.Base64
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import app.tauri.Logger
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.PublicKey
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONObject

@InvokeArg
class SignArgs {
    var message: String = ""
    var reason: String = ""
}

@InvokeArg
class StoreSessionArgs {
    var certDer: String = ""
    var keyPkcs8: String = ""
}

@TauriPlugin
class HeadstateKeysPlugin(private val activity: Activity) : Plugin(activity) {
    companion object {
        private const val KEYSTORE = "AndroidKeyStore"
        private const val ECDSA_ALIAS = "headstate-stepup-ecdsa-p256"
        private const val MLDSA_ALIAS = "headstate-stepup-mldsa-65"
        private const val WRAP_ALIAS = "headstate-session-wrap"
        private const val PREFS = "com.pktstorm.headstate.keys"
        private const val PREF_SESSION = "session"

        /// Documented values; see the header comment.
        private const val ML_DSA_65_KEY_ALGORITHM = "ML-DSA-65"
        private const val ML_DSA_SIGNATURE_ALGORITHM = "ML-DSA"
        private const val ML_DSA_MIN_SDK = 37
        private const val KEYMINT_WITH_ML_DSA = 500

        private const val ECDSA_P256_LEN = 65
        private const val MLDSA_65_LEN = 1952
        private const val MLDSA_WINDOW_SECS = 10
        private const val GCM_IV_LEN = 12
        private const val GCM_TAG_BITS = 128

        // Rejection codes the Rust side maps to `Error` variants.
        private const val CODE_NOT_GENERATED = "notGenerated"
        private const val CODE_CANCELLED = "cancelled"
        private const val CODE_AUTH_FAILED = "authFailed"
        private const val CODE_UNAVAILABLE = "unavailable"
        private const val CODE_MALFORMED = "malformed"
    }

    // ---- Commands ---------------------------------------------------

    @Command
    fun generate(invoke: Invoke) {
        try {
            // Replace, never add: a half-generated set from a failed
            // earlier attempt must not survive next to a new one.
            destroyAll()
            val ecdsa = generateEcdsa()
            val mldsa = generateMldsa()
            Logger.info("headstate-keys: generated step-up keys (ML-DSA-65: ${mldsa != null})")
            invoke.resolve(publicKeysObject(ecdsa, mldsa))
        } catch (e: Exception) {
            invoke.reject("could not create the signing keys: ${e.message}", CODE_UNAVAILABLE, e)
        }
    }

    @Command
    fun publicKeys(invoke: Invoke) {
        try {
            val ks = keyStore()
            val ecdsa = ks.getCertificate(ECDSA_ALIAS)?.publicKey
            if (ecdsa == null) {
                invoke.reject("no device keys", CODE_NOT_GENERATED)
                return
            }
            val mldsa = ks.getCertificate(MLDSA_ALIAS)?.publicKey
            invoke.resolve(
                publicKeysObject(sec1Uncompressed(ecdsa as ECPublicKey), mldsa?.let { rawMldsa(it) })
            )
        } catch (e: Exception) {
            invoke.reject("could not read the public keys: ${e.message}", e)
        }
    }

    @Command
    fun sign(invoke: Invoke) {
        val args = invoke.parseArgs(SignArgs::class.java)
        val message = try {
            Base64.decode(args.message, Base64.DEFAULT)
        } catch (e: IllegalArgumentException) {
            invoke.reject("message is not base64", CODE_MALFORMED)
            return
        }
        val ks = keyStore()
        val ecdsa = ks.getKey(ECDSA_ALIAS, null) as? PrivateKey
        if (ecdsa == null) {
            invoke.reject("no device keys", CODE_NOT_GENERATED)
            return
        }
        val mldsa = ks.getKey(MLDSA_ALIAS, null) as? PrivateKey

        // Per-operation key: initialise first, then authenticate THIS
        // operation through the crypto object.
        val ecdsaOp = Signature.getInstance("SHA256withECDSAinP1363Format")
        try {
            ecdsaOp.initSign(ecdsa)
        } catch (e: KeyPermanentlyInvalidatedException) {
            invoke.reject("the signing key was invalidated; re-pair this phone", CODE_AUTH_FAILED, e)
            return
        }

        prompt(args.reason, BiometricPrompt.CryptoObject(ecdsaOp), invoke) { result ->
            val bound = result.cryptoObject?.signature ?: ecdsaOp
            bound.update(message)
            val ecdsaSig = bound.sign()
            if (mldsa == null) {
                invoke.resolve(signaturesObject(ecdsaSig, null))
                return@prompt
            }
            try {
                invoke.resolve(signaturesObject(ecdsaSig, signMldsa(mldsa, message)))
            } catch (e: UserNotAuthenticatedException) {
                // The bound authentication did not open the ML-DSA key's
                // window on this device. One more prompt, unbound, then
                // sign inside the fresh window.
                Logger.warn("headstate-keys: ML-DSA key needed its own prompt")
                prompt(args.reason, null, invoke) {
                    invoke.resolve(signaturesObject(ecdsaSig, signMldsa(mldsa, message)))
                }
            }
        }
    }

    @Command
    fun destroy(invoke: Invoke) {
        try {
            destroyAll()
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("could not delete the device keys: ${e.message}", e)
        }
    }

    @Command
    fun storeSession(invoke: Invoke) {
        val args = invoke.parseArgs(StoreSessionArgs::class.java)
        try {
            val plaintext = JSONObject()
                .put("certDer", args.certDer)
                .put("keyPkcs8", args.keyPkcs8)
                .toString()
                .toByteArray(Charsets.UTF_8)
            val sealed = Base64.encodeToString(wrap(plaintext), Base64.NO_WRAP)
            prefs().edit().putString(PREF_SESSION, sealed).apply()
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("could not store the session identity: ${e.message}", e)
        }
    }

    @Command
    fun loadSession(invoke: Invoke) {
        val sealed = prefs().getString(PREF_SESSION, null)
        if (sealed == null) {
            invoke.reject("no session identity", CODE_NOT_GENERATED)
            return
        }
        try {
            val json = JSONObject(String(unwrap(Base64.decode(sealed, Base64.NO_WRAP)), Charsets.UTF_8))
            invoke.resolve(
                JSObject()
                    .put("certDer", json.getString("certDer"))
                    .put("keyPkcs8", json.getString("keyPkcs8")) as JSObject
            )
        } catch (e: Exception) {
            invoke.reject("could not read the session identity: ${e.message}", e)
        }
    }

    // ---- Step-up keys -----------------------------------------------

    private fun keyStore(): KeyStore = KeyStore.getInstance(KEYSTORE).also { it.load(null) }

    private fun prefs() = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /// `timeoutSecs == 0` is "every use, through a CryptoObject".
    private fun KeyGenParameterSpec.Builder.authenticated(timeoutSecs: Int): KeyGenParameterSpec.Builder {
        setUserAuthenticationRequired(true)
        setInvalidatedByBiometricEnrollment(false)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            setUserAuthenticationParameters(
                timeoutSecs,
                KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL
            )
        } else {
            // Pre-30: -1 is per-use and biometric-only; a positive value
            // is a window opened by any authentication.
            @Suppress("DEPRECATION")
            setUserAuthenticationValidityDurationSeconds(if (timeoutSecs == 0) -1 else timeoutSecs)
        }
        return this
    }

    private fun generateEcdsa(): ByteArray {
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, KEYSTORE)
        generator.initialize(
            KeyGenParameterSpec.Builder(ECDSA_ALIAS, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .authenticated(0)
                .build()
        )
        val pair = generator.generateKeyPair()
        if (!inSecureHardware(pair.private)) {
            // Kept anyway: there is no better place on this device, and
            // the desktop cannot tell. Logged so the walkthrough can.
            Logger.warn("headstate-keys: the ECDSA step-up key is not in secure hardware")
        }
        return sec1Uncompressed(pair.public as ECPublicKey)
    }

    /// The ML-DSA-65 public key, or null when this device cannot hold
    /// the key in hardware. Never throws.
    private fun generateMldsa(): ByteArray? {
        if (Build.VERSION.SDK_INT < ML_DSA_MIN_SDK) return null
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
            !activity.packageManager.hasSystemFeature(PackageManager.FEATURE_HARDWARE_KEYSTORE, KEYMINT_WITH_ML_DSA)
        ) {
            Logger.info("headstate-keys: no KeyMint 5 keystore; pairing without ML-DSA-65")
            return null
        }
        return try {
            val generator = KeyPairGenerator.getInstance(ML_DSA_65_KEY_ALGORITHM, KEYSTORE)
            generator.initialize(
                KeyGenParameterSpec.Builder(MLDSA_ALIAS, KeyProperties.PURPOSE_SIGN)
                    .setDigests(KeyProperties.DIGEST_NONE)
                    .authenticated(MLDSA_WINDOW_SECS)
                    .build()
            )
            val pair = generator.generateKeyPair()
            if (!inSecureHardware(pair.private)) {
                Logger.warn("headstate-keys: ML-DSA-65 key is not in secure hardware; discarding it")
                deleteAlias(MLDSA_ALIAS)
                return null
            }
            rawMldsa(pair.public)
        } catch (e: Exception) {
            Logger.info("headstate-keys: no ML-DSA-65 key on this device: $e")
            runCatching { deleteAlias(MLDSA_ALIAS) }
            null
        }
    }

    private fun inSecureHardware(key: PrivateKey): Boolean {
        val info = KeyFactory.getInstance(key.algorithm, KEYSTORE).getKeySpec(key, KeyInfo::class.java)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            info.securityLevel == KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT ||
                info.securityLevel == KeyProperties.SECURITY_LEVEL_STRONGBOX
        } else {
            @Suppress("DEPRECATION")
            info.isInsideSecureHardware
        }
    }

    /// Pure ML-DSA, no pre-hash, empty context: the JCA `Signature` for
    /// an Android Keystore ML-DSA key signs exactly that.
    private fun signMldsa(key: PrivateKey, message: ByteArray): ByteArray {
        val op = Signature.getInstance(ML_DSA_SIGNATURE_ALGORITHM)
        op.initSign(key)
        op.update(message)
        return op.sign()
    }

    private fun deleteAlias(alias: String) {
        val ks = keyStore()
        if (ks.containsAlias(alias)) ks.deleteEntry(alias)
    }

    private fun destroyAll() {
        deleteAlias(ECDSA_ALIAS)
        deleteAlias(MLDSA_ALIAS)
        deleteAlias(WRAP_ALIAS)
        prefs().edit().remove(PREF_SESSION).apply()
    }

    // ---- The prompt -------------------------------------------------

    /// Shows the system prompt and, on success, runs `then` off the
    /// callback; any exception from `then` rejects the invoke. Crypto
    /// objects with the device credential need API 30; below that the
    /// key is biometric-only anyway (see `authenticated`).
    private fun prompt(
        reason: String,
        crypto: BiometricPrompt.CryptoObject?,
        invoke: Invoke,
        then: (BiometricPrompt.AuthenticationResult) -> Unit
    ) {
        val host = activity as? FragmentActivity
        if (host == null) {
            invoke.reject("the activity cannot host a biometric prompt", CODE_UNAVAILABLE)
            return
        }
        val allowed = if (crypto != null && Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            BIOMETRIC_STRONG
        } else {
            BIOMETRIC_STRONG or DEVICE_CREDENTIAL
        }
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle("Headstate Companion")
            .setSubtitle(reason)
            .setAllowedAuthenticators(allowed)
            .setConfirmationRequired(false)
            .build()
        val callback = object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                try {
                    then(result)
                } catch (e: Exception) {
                    invoke.reject("could not sign: ${e.message}", CODE_AUTH_FAILED, e)
                }
            }

            override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                val code = when (errorCode) {
                    BiometricPrompt.ERROR_USER_CANCELED,
                    BiometricPrompt.ERROR_NEGATIVE_BUTTON,
                    BiometricPrompt.ERROR_CANCELED -> CODE_CANCELLED
                    else -> CODE_AUTH_FAILED
                }
                invoke.reject(errString.toString(), code)
            }

            // onAuthenticationFailed is a wrong finger or face; the
            // sheet stays up and retries, so there is nothing to do.
        }
        activity.runOnUiThread {
            val p = BiometricPrompt(host, ContextCompat.getMainExecutor(activity), callback)
            if (crypto != null) p.authenticate(info, crypto) else p.authenticate(info)
        }
    }

    // ---- Encodings --------------------------------------------------

    private fun publicKeysObject(ecdsa: ByteArray, mldsa: ByteArray?): JSObject {
        val out = JSObject()
        out.put("ecdsaP256", Base64.encodeToString(ecdsa, Base64.NO_WRAP))
        out.put("mldsa65", mldsa?.let { Base64.encodeToString(it, Base64.NO_WRAP) } ?: JSONObject.NULL)
        return out
    }

    private fun signaturesObject(ecdsa: ByteArray, mldsa: ByteArray?): JSObject {
        val out = JSObject()
        out.put("ecdsa", Base64.encodeToString(ecdsa, Base64.NO_WRAP))
        out.put("mldsa", mldsa?.let { Base64.encodeToString(it, Base64.NO_WRAP) } ?: JSONObject.NULL)
        return out
    }

    /// SEC1 uncompressed: 0x04 || x || y, each coordinate exactly 32
    /// bytes. `BigInteger.toByteArray` is two's complement, so it may
    /// carry a leading zero or be short; both are normalised here.
    private fun sec1Uncompressed(key: ECPublicKey): ByteArray {
        val out = ByteArray(ECDSA_P256_LEN)
        out[0] = 0x04
        fixed(key.w.affineX.toByteArray(), out, 1)
        fixed(key.w.affineY.toByteArray(), out, 33)
        return out
    }

    private fun fixed(coordinate: ByteArray, into: ByteArray, offset: Int) {
        var start = 0
        while (start < coordinate.size - 1 && coordinate[start] == 0.toByte()) start++
        val len = coordinate.size - start
        require(len <= 32) { "coordinate is $len bytes" }
        System.arraycopy(coordinate, start, into, offset + 32 - len, len)
    }

    /// The FIPS 204 public key is the payload of the SubjectPublicKeyInfo
    /// BIT STRING, which is the last 1952 bytes of the X.509 encoding.
    private fun rawMldsa(key: PublicKey): ByteArray {
        val encoded = key.encoded
        require(encoded.size > MLDSA_65_LEN) { "ML-DSA public key encoding is ${encoded.size} bytes" }
        return encoded.copyOfRange(encoded.size - MLDSA_65_LEN, encoded.size)
    }

    // ---- The session wrap key ---------------------------------------

    private fun wrapKey(): SecretKey {
        val ks = keyStore()
        (ks.getKey(WRAP_ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(WRAP_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return generator.generateKey()
    }

    /// iv || ciphertext-with-tag.
    private fun wrap(plaintext: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, wrapKey())
        val iv = cipher.iv
        require(iv.size == GCM_IV_LEN)
        return iv + cipher.doFinal(plaintext)
    }

    private fun unwrap(sealed: ByteArray): ByteArray {
        require(sealed.size > GCM_IV_LEN) { "sealed session is too short" }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(
            Cipher.DECRYPT_MODE,
            wrapKey(),
            GCMParameterSpec(GCM_TAG_BITS, sealed, 0, GCM_IV_LEN)
        )
        return cipher.doFinal(sealed, GCM_IV_LEN, sealed.size - GCM_IV_LEN)
    }
}
