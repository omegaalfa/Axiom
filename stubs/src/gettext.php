<?php
// Runtime declarations exported from extension: gettext

namespace {
    function textdomain(?string $domain = NULL): string {}

    function gettext(string $message): string {}

    function _(string $message): string {}

    function dgettext(string $domain, string $message): string {}

    function dcgettext(string $domain, string $message, int $category): string {}

    function bindtextdomain(string $domain, ?string $directory = NULL): string|false {}

    function ngettext(string $singular, string $plural, int $count): string {}

    function dngettext(string $domain, string $singular, string $plural, int $count): string {}

    function dcngettext(string $domain, string $singular, string $plural, int $count, int $category): string {}

    function bind_textdomain_codeset(string $domain, ?string $codeset = NULL): string|false {}

}
