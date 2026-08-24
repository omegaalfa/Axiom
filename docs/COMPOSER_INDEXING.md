# Composer dependency indexing design (item 21)

This is a design note only; vendor indexing is not implemented yet.

## Detection

Treat a project as Composer-enabled when `vendor/composer/` exists beneath the
project root. Prefer the generated `autoload_psr4.php` and
`autoload_classmap.php` files as the authoritative merged mappings. Fall back
to `composer.json` (`autoload.psr-4` and `autoload.classmap`) only when the
generated files are absent.

## Scheduling

Project files and vendor files should remain separate indexing jobs. Vendor
indexing can involve a large number of files, so it should run on the existing
background channel/poll pattern and never block the UI or delay the initial
project index result. A generation number should discard stale vendor results
when the project changes or closes.

## Symbol provenance

Vendor symbols should carry a distinct source/provenance (`Vendor`) rather than
being merged indistinguishably with project symbols. Completion presentation
would then show `Vendor`, alongside the existing `Runtime` and `Project`
labels. De-duplication should prefer project symbols over vendor symbols when
both resolve to the same fully-qualified name.
