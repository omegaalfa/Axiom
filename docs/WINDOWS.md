# Windows graphics requirements

Axiom uses the Windows renderer supplied by GPUI 0.2.2. The relevant GPUI
implementation is `platform/windows/directx_devices.rs`.

## Adapter selection

GPUI creates a DXGI factory and enumerates adapters with
`IDXGIFactory6::EnumAdapters`, starting at index zero. It selects the first
adapter for which `D3D11CreateDevice` succeeds. There is no NVIDIA, RTX, CUDA,
or vendor-specific filter in this path, so Intel and AMD adapters are eligible
when their driver supports the requested Direct3D feature levels.

GPUI requests feature levels 11.1, 11.0, and 10.1, in that order. The source
does not enumerate a WARP adapter or call `D3D11CreateDevice` with
`D3D_DRIVER_TYPE_WARP`; there is no software-renderer fallback in this GPUI
version.

If no enumerated adapter can create a device at one of those levels, the
renderer initialization returns an error. Axiom currently does not wrap GPUI's
startup with a dedicated graphics error screen, so the final user-facing
failure remains controlled by the GPUI/Application startup path.

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
feature-level, and fallback fields without changing GPUI itself.
