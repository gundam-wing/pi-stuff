{ pkgs, ... }:

let
  camera-snapshot = pkgs.writeShellApplication {
    name = "camera-snapshot";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.imagemagick
      pkgs.libcamera
    ];
    text = ''
      output="''${1:-camera-$(date +%Y%m%d-%H%M%S).jpg}"
      work_dir="$(mktemp -d)"
      trap 'rm -rf "$work_dir"' EXIT

      # Give automatic exposure and white balance a few frames to settle.
      cam \
        --camera 1 \
        --capture=15 \
        --stream role=viewfinder,width=1280,height=720,pixelformat=BGR888 \
        --file="$work_dir/frame-#.ppm"

      last_frame=
      for frame in "$work_dir"/frame-*.ppm; do
        last_frame="$frame"
      done

      magick "$last_frame" "$output"
      printf 'Saved %s\n' "$output"
    '';
  };
in
{
  # Camera Module 3 uses the Sony IMX708 sensor. The Raspberry Pi kernel
  # includes its overlay, but NixOS does not apply it automatically.
  hardware.deviceTree.overlays = [
    {
      name = "imx708";
      # The firmware's precompiled overlay uses parameter fixups that the
      # generic NixOS overlay merger cannot resolve. This is the equivalent
      # fixed Camera Module 3 configuration without those firmware-only
      # parameters.
      dtsText = ''
        /dts-v1/;
        /plugin/;

        / {
          compatible = "raspberrypi,4-model-b";

          fragment@0 {
            target = <&i2c0if>;
            __overlay__ { status = "okay"; };
          };

          fragment@1 {
            target = <&cam1_clk>;
            __overlay__ {
              status = "okay";
              clock-frequency = <24000000>;
            };
          };

          fragment@2 {
            target = <&i2c0mux>;
            __overlay__ { status = "okay"; };
          };

          fragment@3 {
            target = <&cam1_reg>;
            __overlay__ {
              startup-delay-us = <70000>;
              off-on-delay-us = <30000>;
              regulator-min-microvolt = <2700000>;
              regulator-max-microvolt = <2700000>;
            };
          };

          fragment@100 {
            target = <&i2c_csi_dsi>;
            __overlay__ {
              #address-cells = <1>;
              #size-cells = <0>;
              status = "okay";

              cam_node: imx708@1a {
                compatible = "sony,imx708";
                reg = <0x1a>;
                status = "okay";
                clocks = <&cam1_clk>;
                clock-names = "inclk";
                vana1-supply = <&cam1_reg>;
                vana2-supply = <&cam_dummy_reg>;
                vdig-supply = <&cam_dummy_reg>;
                vddl-supply = <&cam_dummy_reg>;
                rotation = <180>;
                orientation = <2>;
                lens-focus = <&vcm_node>;

                port {
                  cam_endpoint: endpoint {
                    clock-lanes = <0>;
                    data-lanes = <1 2>;
                    clock-noncontinuous;
                    link-frequencies = /bits/ 64 <450000000>;
                    remote-endpoint = <&csi_ep>;
                  };
                };
              };

              vcm_node: dw9817@c {
                compatible = "dongwoon,dw9817-vcm";
                reg = <0x0c>;
                status = "okay";
                VDD-supply = <&cam1_reg>;
              };
            };
          };

          fragment@101 {
            target = <&csi1>;
            __overlay__ {
              status = "okay";
              port {
                csi_ep: endpoint {
                  remote-endpoint = <&cam_endpoint>;
                  clock-lanes = <0>;
                  data-lanes = <1 2>;
                  clock-noncontinuous;
                };
              };
            };
          };
        };
      '';
    }
  ];

  # libcamera's Raspberry Pi pipeline allocates image buffers from CMA and
  # dma_heap. Give it enough contiguous memory and permit the video group to
  # use the allocator.
  boot.kernelParams = [ "cma=256M" ];
  services.udev.extraRules = ''
    SUBSYSTEM=="dma_heap", GROUP="video", MODE="0660"
  '';

  environment.systemPackages = with pkgs; [
    camera-snapshot
    libcamera
    v4l-utils
  ];
}
