<?php
// Runtime declarations exported from extension: openssl

namespace {
    function openssl_x509_export_to_file(\OpenSSLCertificate|string $certificate, string $output_filename, bool $no_text = true): bool {}

    function openssl_x509_export(\OpenSSLCertificate|string $certificate, &$output, bool $no_text = true): bool {}

    function openssl_x509_fingerprint(\OpenSSLCertificate|string $certificate, string $digest_algo = 'sha1', bool $binary = false): string|false {}

    function openssl_x509_check_private_key(\OpenSSLCertificate|string $certificate, $private_key): bool {}

    function openssl_x509_verify(\OpenSSLCertificate|string $certificate, $public_key): int {}

    function openssl_x509_parse(\OpenSSLCertificate|string $certificate, bool $short_names = true): array|false {}

    function openssl_x509_checkpurpose(\OpenSSLCertificate|string $certificate, int $purpose, array $ca_info = array (), ?string $untrusted_certificates_file = NULL): int|bool {}

    function openssl_x509_read(\OpenSSLCertificate|string $certificate): \OpenSSLCertificate|false {}

    function openssl_x509_free(\OpenSSLCertificate $certificate): void {}

    function openssl_pkcs12_export_to_file(\OpenSSLCertificate|string $certificate, string $output_filename, $private_key, string $passphrase, array $options = array ()): bool {}

    function openssl_pkcs12_export(\OpenSSLCertificate|string $certificate, &$output, $private_key, string $passphrase, array $options = array ()): bool {}

    function openssl_pkcs12_read(string $pkcs12, &$certificates, string $passphrase): bool {}

    function openssl_csr_export_to_file(\OpenSSLCertificateSigningRequest|string $csr, string $output_filename, bool $no_text = true): bool {}

    function openssl_csr_export(\OpenSSLCertificateSigningRequest|string $csr, &$output, bool $no_text = true): bool {}

    function openssl_csr_sign(\OpenSSLCertificateSigningRequest|string $csr, \OpenSSLCertificate|string|null $ca_certificate, $private_key, int $days, ?array $options = NULL, int $serial = 0, ?string $serial_hex = NULL): \OpenSSLCertificate|false {}

    function openssl_csr_new(array $distinguished_names, &$private_key, ?array $options = NULL, ?array $extra_attributes = NULL): \OpenSSLCertificateSigningRequest|bool {}

    function openssl_csr_get_subject(\OpenSSLCertificateSigningRequest|string $csr, bool $short_names = true): array|false {}

    function openssl_csr_get_public_key(\OpenSSLCertificateSigningRequest|string $csr, bool $short_names = true): \OpenSSLAsymmetricKey|false {}

    function openssl_pkey_new(?array $options = NULL): \OpenSSLAsymmetricKey|false {}

    function openssl_pkey_export_to_file($key, string $output_filename, ?string $passphrase = NULL, ?array $options = NULL): bool {}

    function openssl_pkey_export($key, &$output, ?string $passphrase = NULL, ?array $options = NULL): bool {}

    function openssl_pkey_get_public($public_key): \OpenSSLAsymmetricKey|false {}

    function openssl_get_publickey($public_key): \OpenSSLAsymmetricKey|false {}

    function openssl_pkey_free(\OpenSSLAsymmetricKey $key): void {}

    function openssl_free_key(\OpenSSLAsymmetricKey $key): void {}

    function openssl_pkey_get_private($private_key, ?string $passphrase = NULL): \OpenSSLAsymmetricKey|false {}

    function openssl_get_privatekey($private_key, ?string $passphrase = NULL): \OpenSSLAsymmetricKey|false {}

    function openssl_pkey_get_details(\OpenSSLAsymmetricKey $key): array|false {}

    function openssl_pbkdf2(string $password, string $salt, int $key_length, int $iterations, string $digest_algo = 'sha1'): string|false {}

    function openssl_pkcs7_verify(string $input_filename, int $flags, ?string $signers_certificates_filename = NULL, array $ca_info = array (), ?string $untrusted_certificates_filename = NULL, ?string $content = NULL, ?string $output_filename = NULL): int|bool {}

    function openssl_pkcs7_encrypt(string $input_filename, string $output_filename, $certificate, ?array $headers, int $flags = 0, int $cipher_algo = \OPENSSL_CIPHER_AES_128_CBC): bool {}

    function openssl_pkcs7_sign(string $input_filename, string $output_filename, \OpenSSLCertificate|string $certificate, $private_key, ?array $headers, int $flags = \PKCS7_DETACHED, ?string $untrusted_certificates_filename = NULL): bool {}

    function openssl_pkcs7_decrypt(string $input_filename, string $output_filename, $certificate, $private_key = NULL): bool {}

    function openssl_pkcs7_read(string $data, &$certificates): bool {}

    function openssl_cms_verify(string $input_filename, int $flags = 0, ?string $certificates = NULL, array $ca_info = array (), ?string $untrusted_certificates_filename = NULL, ?string $content = NULL, ?string $pk7 = NULL, ?string $sigfile = NULL, int $encoding = \OPENSSL_ENCODING_SMIME): bool {}

    function openssl_cms_encrypt(string $input_filename, string $output_filename, $certificate, ?array $headers, int $flags = 0, int $encoding = \OPENSSL_ENCODING_SMIME, string|int $cipher_algo = \OPENSSL_CIPHER_AES_128_CBC): bool {}

    function openssl_cms_sign(string $input_filename, string $output_filename, \OpenSSLCertificate|string $certificate, $private_key, ?array $headers, int $flags = 0, int $encoding = \OPENSSL_ENCODING_SMIME, ?string $untrusted_certificates_filename = NULL): bool {}

    function openssl_cms_decrypt(string $input_filename, string $output_filename, $certificate, $private_key = NULL, int $encoding = \OPENSSL_ENCODING_SMIME): bool {}

    function openssl_cms_read(string $input_filename, &$certificates): bool {}

    function openssl_private_encrypt(string $data, &$encrypted_data, $private_key, int $padding = \OPENSSL_PKCS1_PADDING): bool {}

    function openssl_private_decrypt(string $data, &$decrypted_data, $private_key, int $padding = \OPENSSL_PKCS1_PADDING, ?string $digest_algo = NULL): bool {}

    function openssl_public_encrypt(string $data, &$encrypted_data, $public_key, int $padding = \OPENSSL_PKCS1_PADDING, ?string $digest_algo = NULL): bool {}

    function openssl_public_decrypt(string $data, &$decrypted_data, $public_key, int $padding = \OPENSSL_PKCS1_PADDING): bool {}

    function openssl_error_string(): string|false {}

    function openssl_sign(string $data, &$signature, $private_key, string|int $algorithm = \OPENSSL_ALGO_SHA1, int $padding = 0): bool {}

    function openssl_verify(string $data, string $signature, $public_key, string|int $algorithm = \OPENSSL_ALGO_SHA1, int $padding = 0): int|false {}

    function openssl_seal(string $data, &$sealed_data, &$encrypted_keys, array $public_key, string $cipher_algo, &$iv = NULL): int|false {}

    function openssl_open(string $data, &$output, string $encrypted_key, $private_key, string $cipher_algo, ?string $iv = NULL): bool {}

    function openssl_get_md_methods(bool $aliases = false): array {}

    function openssl_get_cipher_methods(bool $aliases = false): array {}

    function openssl_get_curve_names(): array|false {}

    function openssl_digest(string $data, string $digest_algo, bool $binary = false): string|false {}

    function openssl_encrypt(string $data, string $cipher_algo, string $passphrase, int $options = 0, string $iv = '', &$tag = NULL, string $aad = '', int $tag_length = 16): string|false {}

    function openssl_decrypt(string $data, string $cipher_algo, string $passphrase, int $options = 0, string $iv = '', ?string $tag = NULL, string $aad = ''): string|false {}

    function openssl_cipher_iv_length(string $cipher_algo): int|false {}

    function openssl_cipher_key_length(string $cipher_algo): int|false {}

    function openssl_dh_compute_key(string $public_key, \OpenSSLAsymmetricKey $private_key): string|false {}

    function openssl_pkey_derive($public_key, $private_key, int $key_length = 0): string|false {}

    function openssl_random_pseudo_bytes(int $length, &$strong_result = NULL): string {}

    function openssl_spki_new(\OpenSSLAsymmetricKey $private_key, string $challenge, int $digest_algo = \OPENSSL_ALGO_MD5): string|false {}

    function openssl_spki_verify(string $spki): bool {}

    function openssl_spki_export(string $spki): string|false {}

    function openssl_spki_export_challenge(string $spki): string|false {}

    function openssl_get_cert_locations(): array {}

    final class OpenSSLCertificate {
    }

    final class OpenSSLCertificateSigningRequest {
    }

    final class OpenSSLAsymmetricKey {
    }

    const OPENSSL_VERSION_TEXT = 'OpenSSL 3.5.5 27 Jan 2026';

    const OPENSSL_VERSION_NUMBER = 810549328;

    const X509_PURPOSE_SSL_CLIENT = 1;

    const X509_PURPOSE_SSL_SERVER = 2;

    const X509_PURPOSE_NS_SSL_SERVER = 3;

    const X509_PURPOSE_SMIME_SIGN = 4;

    const X509_PURPOSE_SMIME_ENCRYPT = 5;

    const X509_PURPOSE_CRL_SIGN = 6;

    const X509_PURPOSE_ANY = 7;

    const X509_PURPOSE_OCSP_HELPER = 8;

    const X509_PURPOSE_TIMESTAMP_SIGN = 9;

    const OPENSSL_ALGO_SHA1 = 1;

    const OPENSSL_ALGO_MD5 = 2;

    const OPENSSL_ALGO_MD4 = 3;

    const OPENSSL_ALGO_SHA224 = 6;

    const OPENSSL_ALGO_SHA256 = 7;

    const OPENSSL_ALGO_SHA384 = 8;

    const OPENSSL_ALGO_SHA512 = 9;

    const OPENSSL_ALGO_RMD160 = 10;

    const PKCS7_DETACHED = 64;

    const PKCS7_TEXT = 1;

    const PKCS7_NOINTERN = 16;

    const PKCS7_NOVERIFY = 32;

    const PKCS7_NOCHAIN = 8;

    const PKCS7_NOCERTS = 2;

    const PKCS7_NOATTR = 256;

    const PKCS7_BINARY = 128;

    const PKCS7_NOSIGS = 4;

    const PKCS7_NOOLDMIMETYPE = 1024;

    const PKCS7_NOSMIMECAP = 512;

    const PKCS7_CRLFEOL = 2048;

    const PKCS7_NOCRL = 8192;

    const PKCS7_NO_DUAL_CONTENT = 65536;

    const OPENSSL_CMS_DETACHED = 64;

    const OPENSSL_CMS_TEXT = 1;

    const OPENSSL_CMS_NOINTERN = 16;

    const OPENSSL_CMS_NOVERIFY = 32;

    const OPENSSL_CMS_NOCERTS = 2;

    const OPENSSL_CMS_NOATTR = 256;

    const OPENSSL_CMS_BINARY = 128;

    const OPENSSL_CMS_NOSIGS = 12;

    const OPENSSL_CMS_OLDMIMETYPE = 1024;

    const OPENSSL_PKCS1_PADDING = 1;

    const OPENSSL_NO_PADDING = 3;

    const OPENSSL_PKCS1_OAEP_PADDING = 4;

    const OPENSSL_PKCS1_PSS_PADDING = 6;

    const OPENSSL_DEFAULT_STREAM_CIPHERS = 'ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-AES256-GCM-SHA384:DHE-RSA-AES128-GCM-SHA256:DHE-DSS-AES128-GCM-SHA256:kEDH+AESGCM:ECDHE-RSA-AES128-SHA256:ECDHE-ECDSA-AES128-SHA256:ECDHE-RSA-AES128-SHA:ECDHE-ECDSA-AES128-SHA:ECDHE-RSA-AES256-SHA384:ECDHE-ECDSA-AES256-SHA384:ECDHE-RSA-AES256-SHA:ECDHE-ECDSA-AES256-SHA:DHE-RSA-AES128-SHA256:DHE-RSA-AES128-SHA:DHE-DSS-AES128-SHA256:DHE-RSA-AES256-SHA256:DHE-DSS-AES256-SHA:DHE-RSA-AES256-SHA:AES128-GCM-SHA256:AES256-GCM-SHA384:AES128:AES256:HIGH:!SSLv2:!aNULL:!eNULL:!EXPORT:!DES:!MD5:!RC4:!ADH';

    const OPENSSL_CIPHER_RC2_40 = 0;

    const OPENSSL_CIPHER_RC2_128 = 1;

    const OPENSSL_CIPHER_RC2_64 = 2;

    const OPENSSL_CIPHER_DES = 3;

    const OPENSSL_CIPHER_3DES = 4;

    const OPENSSL_CIPHER_AES_128_CBC = 5;

    const OPENSSL_CIPHER_AES_192_CBC = 6;

    const OPENSSL_CIPHER_AES_256_CBC = 7;

    const OPENSSL_KEYTYPE_RSA = 0;

    const OPENSSL_KEYTYPE_DSA = 1;

    const OPENSSL_KEYTYPE_DH = 2;

    const OPENSSL_KEYTYPE_EC = 3;

    const OPENSSL_KEYTYPE_X25519 = 4;

    const OPENSSL_KEYTYPE_ED25519 = 5;

    const OPENSSL_KEYTYPE_X448 = 6;

    const OPENSSL_KEYTYPE_ED448 = 7;

    const OPENSSL_RAW_DATA = 1;

    const OPENSSL_ZERO_PADDING = 2;

    const OPENSSL_DONT_ZERO_PAD_KEY = 4;

    const OPENSSL_TLSEXT_SERVER_NAME = 1;

    const OPENSSL_ENCODING_DER = 0;

    const OPENSSL_ENCODING_SMIME = 1;

    const OPENSSL_ENCODING_PEM = 2;

}
