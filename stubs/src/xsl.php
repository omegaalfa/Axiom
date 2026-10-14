<?php
// Runtime declarations exported from extension: xsl

namespace {
    class XSLTProcessor {
        public  bool $doXInclude = false;
        public  bool $cloneDocument = false;
        public  int $maxTemplateDepth;
        public  int $maxTemplateVars;
        public function importStylesheet(object $stylesheet) {}
        public function transformToDoc(object $document, ?string $returnClass = NULL) {}
        public function transformToUri(object $document, string $uri) {}
        public function transformToXml(object $document) {}
        public function setParameter(string $namespace, array|string $name, ?string $value = NULL) {}
        public function getParameter(string $namespace, string $name) {}
        public function removeParameter(string $namespace, string $name) {}
        public function hasExsltSupport() {}
        public function registerPHPFunctions(array|string|null $functions = NULL) {}
        public function registerPHPFunctionNS(string $namespaceURI, string $name, callable $callable): void {}
        public function setProfiling(?string $filename) {}
        public function setSecurityPrefs(int $preferences) {}
        public function getSecurityPrefs() {}
    }

    const XSL_CLONE_AUTO = 0;

    const XSL_CLONE_NEVER = -1;

    const XSL_CLONE_ALWAYS = 1;

    const XSL_SECPREF_NONE = 0;

    const XSL_SECPREF_READ_FILE = 2;

    const XSL_SECPREF_WRITE_FILE = 4;

    const XSL_SECPREF_CREATE_DIRECTORY = 8;

    const XSL_SECPREF_READ_NETWORK = 16;

    const XSL_SECPREF_WRITE_NETWORK = 32;

    const XSL_SECPREF_DEFAULT = 44;

    const LIBXSLT_VERSION = 10145;

    const LIBXSLT_DOTTED_VERSION = '1.1.45';

    const LIBEXSLT_VERSION = 825;

    const LIBEXSLT_DOTTED_VERSION = '0.8.25';

}
