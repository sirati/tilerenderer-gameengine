{
  description = "Civ3Renderer: modern tile/menu renderer driven by Civilization III";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      mingw64 = pkgs.pkgsCross.mingwW64;
      mingw32 = pkgs.pkgsCross.mingw32;
      runtimeLibs = with pkgs; [ vulkan-loader wayland libxkbcommon libGL ];
    in {
      devShells.${system}.default = pkgs.mkShell {
        # The toolchain itself comes from rustup (rust-toolchain.toml); the
        # shader build additionally pulls rust-gpu's pinned nightly.
        nativeBuildInputs = with pkgs; [
          rustup
          pkg-config
          gcc
          mingw64.stdenv.cc
          mingw32.stdenv.cc
          spirv-tools
        ];
        buildInputs = runtimeLibs;

        # Windows targets link with mingw-w64 and need its pthreads.
        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "x86_64-w64-mingw32-gcc";
        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L native=${mingw64.windows.pthreads}/lib";
        CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER = "i686-w64-mingw32-gcc";
        # nixpkgs builds the 32-bit mingw gcc with the mcfgthread threading
        # model; libgcc_eh needs it. Linked statically, so no extra DLL has to
        # ship next to the game.
        CARGO_TARGET_I686_PC_WINDOWS_GNU_RUSTFLAGS = "-L native=${mingw32.windows.pthreads}/lib -L native=${mingw32.windows.mcfgthreads}/lib -C link-arg=-Wl,-Bstatic -C link-arg=-lmcfgthread -C link-arg=-Wl,-Bdynamic -C link-arg=-lkernel32 -C link-arg=-lntdll";

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;

        # The cross compilers' setup hooks claim CC/CXX; the host build
        # (build scripts, rust-gpu's spirv-tools) must use the native ones,
        # and cc-rs gets the cross ones per target.
        shellHook = ''
          export CC=gcc CXX=g++
          export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++
          export CC_i686_pc_windows_gnu=i686-w64-mingw32-gcc CXX_i686_pc_windows_gnu=i686-w64-mingw32-g++
          export PATH=$HOME/.cargo/bin:$PATH
          # Tests and tools run on the AMD iGPU only: the Vulkan loader sees
          # no other driver, so nothing can land on the NVIDIA cards (the
          # 32-bit one is for the game's own process under Wine).
          export VK_DRIVER_FILES=/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json:$HOME/.cache/winvm/mesa-i686/share/vulkan/icd.d/radeon_icd.i686.json
          export TILE_RENDER_ADAPTER=''${TILE_RENDER_ADAPTER:-radv}
          export __EGL_VENDOR_LIBRARY_FILENAMES=/run/opengl-driver/share/glvnd/egl_vendor.d/50_mesa.json
          export DRI_PRIME=pci-0000_72_00_0
          export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=$PWD/tools/wine-run
          export CARGO_TARGET_I686_PC_WINDOWS_GNU_RUNNER=$PWD/tools/wine-run
        '';
      };
    };
}
