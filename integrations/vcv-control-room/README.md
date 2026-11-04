# Control Room Bridge 8 for VCV Rack

This Rack 2 plugin exposes eight `SEND` CV inputs and eight `RETURN` CV outputs. Rack voltages in the normal unipolar `0–10 V` control range are normalized to `0–1` for Control Room.

- Rack → Control Room: UDP `127.0.0.1:9100`, `/controlroom/in/1` through `/controlroom/in/8`
- Control Room → Rack: UDP `127.0.0.1:9101`, `/controlroom/out/1` through `/controlroom/out/8`
- Rack Copilot agent: TCP `127.0.0.1:9102`, newline-delimited JSON, localhost only
- The Rack audio callback performs only bounded atomic loads/stores. A worker thread performs all UDP/OSC work at a 10 ms cadence.
- Incoming values are clamped to `0–1`; return ports emit `0–10 V`.

Run Control Room with the companion patch:

```sh
controlroom --dry-run config/vcv-bridge.yaml
```

Install the SDK matching the installed Rack version, then build and install:

```sh
scripts/install-vcv-sdk.sh
make -C integrations/vcv-control-room RACK_DIR=../../target/vcv-sdk/Rack-SDK dist
make -C integrations/vcv-control-room RACK_DIR=../../target/vcv-sdk/Rack-SDK install
```

Run the complete build/load/bidirectional transport proof with:

```sh
scripts/verify-vcv-bridge.sh
```

The bridge is verified with VCV Rack Pro 2.6.6 and the matching official macOS ARM64 SDK. The verifier uses an isolated Rack user directory and a one-module headless patch, so it does not change the user's Rack patch or settings. The transport-independent OSC codec can also be tested alone with:

```sh
clang++ -std=c++17 -pthread \
  integrations/vcv-control-room/src/OscBridge.cpp \
  integrations/vcv-control-room/tests/osc_codec_test.cpp \
  -o target/vcv-osc-codec-test
target/vcv-osc-codec-test
```

The Copilot agent deliberately separates network and Rack work. Its TCP thread only transports bounded JSON messages. Inventory and graph commands are processed from `Bridge8Widget.step()` on Rack's UI thread; no graph mutation or JSON allocation occurs in the audio callback. Snapshot returns the installed model catalog plus the current patch and a deterministic fingerprint. Apply accepts only `add_module`, `set_param`, and `connect`, re-validates all model/port/parameter references, pushes one native Rack history action, and rolls back if mutation fails. Undo refuses to run if any user action followed the Copilot action.

Because graph work is UI-thread-owned, the agent is available while Rack runs normally, not in Rack's DSP-only `-h` mode. `scripts/rack-agent-probe.py smoke` can exercise live snapshot, Apply, and Undo against an open isolated Rack session. The provider-facing Control Room service is independently covered with mock OpenAI, Claude, and Grok planners plus an ACP v1 negotiation/streaming/permission-cancellation test, so the automated suite requires no API credits.

The manifest currently uses Rack's documented `proprietary` freeware identifier. Before distributing or selling the plugin, choose the final owner/license and review the current Rack plugin and component-library licensing terms. The panel is original and does not copy VCV Core artwork or use the VCV logo.
