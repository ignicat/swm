{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  bash,
  coreutils,
  dbus,
  gnugrep,
  gnused,
  procps,
  systemd,
  systemdLibs,
  libdisplay-info_0_3,
  libdrm,
  libgbm,
  libglvnd,
  libinput,
  libx11,
  libxi,
  libxkbcommon,
  pipewire,
  seatd,
  wayland,
  xwayland,
}:

rustPlatform.buildRustPackage {
  pname = "swm";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;

  src = lib.cleanSourceWith {
    src = lib.cleanSource ../.;
    filter = path: type: !(type == "directory" && builtins.baseNameOf path == "target");
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
    # Both crates use the same unmodified upstream checkout; one hash covers it.
    outputHashes."smithay-0.7.0" = "sha256-YD7P5iZAr+0XdeURhq6DkRkFws6Dpn0zSU5Y9qvsyZQ=";
  };

  nativeBuildInputs = [
    pkg-config
    makeWrapper
    rustPlatform.bindgenHook
  ];
  buildInputs = [
    libdisplay-info_0_3
    libdrm
    libgbm
    libglvnd
    libinput
    libx11
    libxi
    libxkbcommon
    pipewire
    seatd
    systemdLibs
    wayland
  ];
  nativeCheckInputs = [
    bash
    coreutils
    dbus
  ];

  cargoBuildFlags = [ "--workspace" ];
  cargoTestFlags = [ "--workspace" ];

  preCheck = ''
    export XDG_RUNTIME_DIR=$(mktemp -d)
  '';

  # Keep dynamically loaded graphics libraries in the runtime closure/RPATH.
  env.RUSTFLAGS = lib.concatStringsSep " " (
    map (arg: "-C link-arg=${arg}") [
      "-Wl,--push-state,--no-as-needed"
      "-lEGL"
      "-lwayland-client"
      "-lX11"
      "-lX11-xcb"
      "-lXi"
      "-lxkbcommon-x11"
      "-Wl,--pop-state"
    ]
  );

  postPatch = ''
    # These test fixtures deliberately clear PATH and create executable scripts.
    substituteInPlace tests/session_lifecycle.rs \
      --replace-fail '/bin/sh' '${lib.getExe bash}' \
      --replace-fail '/usr/bin:/bin' '${
        lib.makeBinPath [
          bash
          coreutils
        ]
      }'
    substituteInPlace tests/idle_service.rs swm-portal/tests/portal_dispatch.rs \
      --replace-fail '"--session"' '"--config-file=${dbus}/share/dbus-1/session.conf"'
  '';


  postInstall = ''
    install -Dm755 packaging/wayland-sessions/swm-session "$out/bin/swm-session"
    patchShebangs "$out/bin/swm-session"
    substituteInPlace "$out/bin/swm-session" \
      --replace-fail '/usr/bin/swm' "$out/bin/swm"

    install -Dm644 packaging/wayland-sessions/swm.desktop \
      "$out/share/wayland-sessions/swm.desktop"
    substituteInPlace "$out/share/wayland-sessions/swm.desktop" \
      --replace-fail '=swm-session' "=$out/bin/swm-session"

    mkdir -p "$out/share/xdg-desktop-portal" "$out/share/dbus-1/services" \
      "$out/lib/systemd/user"
    cp -r packaging/xdg-desktop-portal/. "$out/share/xdg-desktop-portal/"
    cp packaging/dbus-1/services/*.service "$out/share/dbus-1/services/"
    cp -r packaging/systemd-user/. "$out/lib/systemd/user/"
    substituteInPlace \
      "$out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.swm.service" \
      "$out/lib/systemd/user/xdg-desktop-portal-swm.service" \
      --replace-fail '/usr/bin/xdg-desktop-portal-swm' "$out/bin/xdg-desktop-portal-swm"
    substituteInPlace "$out/lib/systemd/user/swm.service" \
      --replace-fail '/usr/bin/swm-session' "$out/bin/swm-session"

    wrapProgram "$out/bin/swm" \
      --prefix PATH : ${
        lib.makeBinPath [
          bash
          dbus
          systemd
          xwayland
        ]
      }
    wrapProgram "$out/bin/swm-session" \
      --prefix PATH : ${
        lib.makeBinPath [
          coreutils
          dbus
          gnugrep
          gnused
          procps
          systemd
        ]
      }
  '';

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    "$out/bin/swm" --version
    "$out/bin/swmctl" --help
    test -x "$out/bin/xdg-desktop-portal-swm"
    test -x "$out/bin/swm-session"
    grep -F "Exec=$out/bin/swm-session" "$out/share/wayland-sessions/swm.desktop"
    grep -F "ExecStart=$out/bin/swm-session" "$out/lib/systemd/user/swm.service"
    grep -F "ExecStart=$out/bin/xdg-desktop-portal-swm" \
      "$out/lib/systemd/user/xdg-desktop-portal-swm.service"
    runHook postInstallCheck
  '';

  passthru.providedSessions = [ "swm" ];

  meta = {
    description = "simple window manager — spatial Wayland window manager built around per-monitor canvases";
    homepage = "https://github.com/ignicat/swm";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
    mainProgram = "swm";
  };
}
