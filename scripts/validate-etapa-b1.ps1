# Axiom - Script de Validação da Etapa B.1
# Windows-only; run from the repository root.
# Execute este script no PowerShell após as correções para validar a compilação.
#
# Uso:
#   From the repository root:
#   powershell -ExecutionPolicy Bypass -File scripts\validate-etapa-b1.ps1

$ErrorActionPreference = "Stop"
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Axiom - Validação Etapa B.1" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 1. Verificar ambiente
Write-Host "1. Verificando ambiente..." -ForegroundColor Yellow
try {
    $rustc = rustc --version
    $cargo = cargo --version
    Write-Host "   ✓ rustc: $rustc" -ForegroundColor Green
    Write-Host "   ✓ cargo: $cargo" -ForegroundColor Green
} catch {
    Write-Host "   ✗ Rust/Cargo não encontrados no PATH" -ForegroundColor Red
    exit 1
}

Write-Host ""

# 2. Verificar toolchain
Write-Host "2. Verificando toolchain ativo..." -ForegroundColor Yellow
$toolchain = rustup show active-toolchain
Write-Host "   Toolchain: $toolchain" -ForegroundColor Green

Write-Host ""

# 3. Cargo check
Write-Host "3. Executando cargo check --workspace..." -ForegroundColor Yellow
try {
    cargo check --workspace 2>&1 | ForEach-Object { Write-Host "   $_" }
    if ($LASTEXITCODE -eq 0) {
        Write-Host "   ✓ cargo check passou" -ForegroundColor Green
    } else {
        Write-Host "   ✗ cargo check falhou" -ForegroundColor Red
        exit 1
    }
} catch {
    Write-Host "   ✗ Erro ao executar cargo check" -ForegroundColor Red
    exit 1
}

Write-Host ""

# 4. Cargo test
Write-Host "4. Executando cargo test --workspace..." -ForegroundColor Yellow
try {
    cargo test --workspace 2>&1 | ForEach-Object { Write-Host "   $_" }
    if ($LASTEXITCODE -eq 0) {
        Write-Host "   ✓ cargo test passou" -ForegroundColor Green
    } else {
        Write-Host "   ✗ cargo test falhou" -ForegroundColor Red
        exit 1
    }
} catch {
    Write-Host "   ✗ Erro ao executar cargo test" -ForegroundColor Red
    exit 1
}

Write-Host ""

# 5. Cargo clippy
Write-Host "5. Executando cargo clippy..." -ForegroundColor Yellow
try {
    cargo clippy --workspace --all-targets --all-features 2>&1 | ForEach-Object { Write-Host "   $_" }
    if ($LASTEXITCODE -eq 0) {
        Write-Host "   ✓ cargo clippy passou" -ForegroundColor Green
    } else {
        Write-Host "   ⚠ cargo clippy tem warnings (não bloqueante)" -ForegroundColor Yellow
    }
} catch {
    Write-Host "   ⚠ Erro ao executar cargo clippy (não bloqueante)" -ForegroundColor Yellow
}

Write-Host ""

# 6. Cargo build --release
Write-Host "6. Executando cargo build --release..." -ForegroundColor Yellow
try {
    cargo build --release 2>&1 | ForEach-Object { Write-Host "   $_" }
    if ($LASTEXITCODE -eq 0) {
        Write-Host "   ✓ cargo build --release passou" -ForegroundColor Green
    } else {
        Write-Host "   ✗ cargo build --release falhou" -ForegroundColor Red
        exit 1
    }
} catch {
    Write-Host "   ✗ Erro ao executar cargo build --release" -ForegroundColor Red
    exit 1
}

Write-Host ""

# 7. Verificar binário
Write-Host "7. Verificando binário gerado..." -ForegroundColor Yellow
$binPath = "target\release\axiom.exe"
if (Test-Path $binPath) {
    $size = (Get-Item $binPath).Length / 1MB
    Write-Host "   ✓ Binário encontrado: $binPath (${size:N2} MB)" -ForegroundColor Green
} else {
    Write-Host "   ✗ Binário não encontrado: $binPath" -ForegroundColor Red
    exit 1
}

Write-Host ""

# 8. Verificar dependências transitivas (não deve ter floem/wgpu)
Write-Host "8. Verificando dependências do axiom-editor..." -ForegroundColor Yellow
try {
    $tree = cargo tree -p axiom-editor --no-dedupe 2>&1
    $hasFloem = $tree -match "floem "
    $hasWgpu = $tree -match "wgpu "
    $hasWinit = $tree -match "winit "
    
    if ($hasFloem) {
        Write-Host "   ✗ Dependência indesejada encontrada: floem (UI framework)" -ForegroundColor Red
    } else {
        Write-Host "   ✓ Nenhuma dependência de floem (UI framework)" -ForegroundColor Green
    }
    
    if ($hasWgpu) {
        Write-Host "   ✗ Dependência indesejada encontrada: wgpu (renderização)" -ForegroundColor Red
    } else {
        Write-Host "   ✓ Nenhuma dependência de wgpu (renderização)" -ForegroundColor Green
    }
    
    if ($hasWinit) {
        Write-Host "   ✗ Dependência indesejada encontrada: winit (windowing)" -ForegroundColor Red
    } else {
        Write-Host "   ✓ Nenhuma dependência de winit (windowing)" -ForegroundColor Green
    }
} catch {
    Write-Host "   ⚠ Não foi possível verificar dependências" -ForegroundColor Yellow
}

Write-Host ""

# 9. Verificar Cargo.lock
Write-Host "9. Verificando Cargo.lock..." -ForegroundColor Yellow
if (Test-Path "Cargo.lock") {
    Write-Host "   ✓ Cargo.lock presente" -ForegroundColor Green
} else {
    Write-Host "   ✗ Cargo.lock não encontrado (deveria ser gerado pelo cargo)" -ForegroundColor Red
}

Write-Host ""

# Resumo final
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Validação concluída" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "Próximos passos:" -ForegroundColor Yellow
Write-Host "  1. Execute: cargo run --release -p axiom-app" -ForegroundColor White
Write-Host "  2. Verifique se a janela Axiom abre" -ForegroundColor White
Write-Host "  3. Verifique se não há panics no console" -ForegroundColor White
Write-Host ""
