{
  stdenv,
  lib,
  fetchurl,
  meson,
  ninja,
  pkg-config,
  cmake,
  boost,
  python3,
  libcamera,
  libexif,
  libjpeg,
  libtiff,
  libpng,
}:

stdenv.mkDerivation (finalAttrs: {
  pname = "rpicam-apps";
  version = "1.13.0";

  src = fetchurl {
    url = "https://github.com/raspberrypi/rpicam-apps/releases/download/v${finalAttrs.version}/rpicam-apps_${finalAttrs.version}.orig.tar.xz";
    hash = "sha256-dHx69pA+4sf/ZTORaRrLRXMekCe8rOx5MOuiNWXWppw=";
  };

  nativeBuildInputs = [
    meson
    ninja
    pkg-config
    cmake
    python3
  ];

  buildInputs = [
    boost
    libcamera
    libexif
    libjpeg
    libtiff
    libpng
  ];

  mesonFlags = [
    "-Denable_libav=disabled"
    "-Denable_drm=disabled"
    "-Denable_egl=disabled"
    "-Denable_wayland=disabled"
    "-Denable_qt=disabled"
    "-Denable_opencv=disabled"
    "-Denable_tflite=disabled"
    "-Denable_hailo=disabled"
    "-Denable_imx500=false"
  ];

  env = {
    BOOST_INCLUDEDIR = "${lib.getDev boost}/include";
    BOOST_LIBRARYDIR = "${lib.getLib boost}/lib";
    NIX_CFLAGS_COMPILE = "-I${lib.getDev boost}/include";
  };

  postPatch = ''
    patchShebangs utils
  '';

  meta = {
    description = "Raspberry Pi camera applications";
    homepage = "https://github.com/raspberrypi/rpicam-apps";
    license = lib.licenses.bsd2;
    platforms = [ "aarch64-linux" ];
  };
})
