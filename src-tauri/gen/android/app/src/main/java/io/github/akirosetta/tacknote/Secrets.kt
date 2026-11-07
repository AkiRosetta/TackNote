package io.github.akirosetta.tacknote

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.io.IOException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The WebDAV password, encrypted with a key that never leaves the Android
 * Keystore. Only the encrypted bytes are stored, in the app's private
 * preferences. Never logs the password.
 */
object Secrets {
  private const val ALIAS = "tacknote-webdav"
  private const val PREFERENCES = "tacknote-secrets"
  private const val ENTRY = "webdav-password"
  private const val IV_BYTES = 12

  private fun key(): SecretKey {
    val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    generator.init(
      KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .build(),
    )
    return generator.generateKey()
  }

  private fun preferences(context: Context) = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

  fun write(context: Context, password: String) {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.ENCRYPT_MODE, key())
    val sealed = cipher.iv + cipher.doFinal(password.toByteArray(Charsets.UTF_8))
    val saved = preferences(context).edit().putString(ENTRY, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()
    if (!saved) throw IOException("the encrypted password could not be saved")
  }

  /** Null when nothing is saved. Fails when it cannot be decrypted, such as after a restore to another phone. */
  fun read(context: Context): String? {
    val stored = preferences(context).getString(ENTRY, null) ?: return null
    val sealed = Base64.decode(stored, Base64.NO_WRAP)
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, sealed, 0, IV_BYTES))
    return String(cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES), Charsets.UTF_8)
  }
}
