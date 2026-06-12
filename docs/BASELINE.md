# Legacy Baseline

The legacy packages are behavioral references only and are not linked into the new product.

| Artifact | Role | SHA-256 |
| --- | --- | --- |
| `JPAgentServiceSetup.zip` | Windows .NET agent installer | `d77b83a4d96ef43683cd6ba7db331a4f1c4343e1e2552b357df62a7ed6992b7a` |
| `JPAgentService_ubuntu.zip` | Linux .NET agent | `c825acba830ad9515c1f66e42bb360c91baf44007e15638759d086a882cc8d1f` |
| `Station_linux_v1.0.12.tar.gz` | Linux Flutter client | `4fb8656da6de6fbffdbd51646a4a4783c46a6589df40c0ad7ced9113877d17b6` |
| `Station_windows_v1.0.12.zip` | Windows Flutter client | `1ab2974bfbff8b75704c99f87753db79af4c463c8438ef5ad334e36199fa1f69` |

Observed legacy architecture:

- Flutter desktop Station 1.0.12.
- .NET 5 `JanusServiceAgent` managed as systemd or Windows Service.
- Native JPC SDK bridge using protobuf over a local named pipe.
- USB, serial CDC ACM, and Wi-Fi device paths.
- Login, permissions, token information, install, remove, ESI recovery, and agent update flows.

Do not repeat the legacy Linux installer's global OpenSSL modifications or HTTP dependency download.

