# Windows graphics requirements

Axiom uses the Windows renderer supplied by GPUI 0.2.2. The relevant GPUI
implementation is `platform/windows/directx_devices.rs`.

## Adapter selection

GPUI creates a DXGI factory and enumerates adapters with
`IDXGIFactory6::EnumAdapters`, starting at index zero. It selects the first
adapter for which `D3D11CreateDevice` succeeds. There is no NVIDIA, RTX, CUDA,
or vendor-specific filter in this path, so Intel and AMD adapters are eligible
when their driver supports the requested Direct3D feature levels.

GPUI requests feature levels 11.1, 11.0, and 10.1, in that order. The
selection loop does not inspect or reject `DXGI_ADAPTER_FLAG_SOFTWARE`, and
the source does not explicitly enumerate WARP or call
`D3D11CreateDevice` with `D3D_DRIVER_TYPE_WARP`. Consequently, the Microsoft
Basic Render Driver could be accepted by the same loop if it is returned by
DXGI and successfully creates a device; GPUI does not label that case as a
software fallback in its logs.

If no enumerated adapter can create a device at one of those levels, the
renderer initialization returns an error. Axiom emits a clear `[GRAPHICS]`
startup message describing the required Direct3D 11 capabilities before the
normal panic/error report. Axiom does not implement its own WARP or alternate
renderer fallback.

## Debug layer

In debug builds GPUI probes `DXGIGetDebugInterface1`. If it is unavailable, it
logs a warning and creates the DXGI factory without `DXGI_CREATE_FACTORY_DEBUG`;
the device is then created without `D3D11_CREATE_DEVICE_DEBUG`. In release
builds the probe is disabled and the debug flag is always false. Therefore a
missing DXGI debug interface is diagnostic-only and does not prevent normal
device creation.

## Validation status

The source audit confirms support is not restricted to dedicated GPUs, but no
Intel/AMD/WARP hardware is available in this environment for runtime testing.
The existing log line reports the selected adapter name; GPUI 0.2.2 does not
expose a public adapter-selection hook that Axiom can use to add vendor,
feature-level, device-type, and fallback fields without changing GPUI itself.

The Axiom binary also does not export `NvOptimusEnablement` or
`AmdPowerXpressRequestHighPerformance`. On hybrid laptops, adapter choice can
therefore still be influenced by Windows or the manufacturer's driver panel.

## Editor and terminal fonts

Code and terminal rendering use the shared Axiom font policy:

1. Cascadia Mono
2. Consolas
3. DejaVu Sans Mono
4. GPUI's platform fallback stack

The first three families are preferences, not distribution requirements. If
Cascadia Mono is absent, DirectWrite/GPUI selects the next available family;
the application does not bundle or install fonts.
