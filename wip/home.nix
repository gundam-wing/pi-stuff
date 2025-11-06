{ config, pkgs, ... }:

{
  home.stateVersion = "23.11";

  # Enable session management
  xsession = {
    enable = true;
    initExtra = ''
      xterm &
    '';
  };

  # XTerm configuration with Gruvbox Dark theme
  xresources.properties = {
    # General settings
    "XTerm*termName" = "xterm-256color";
    "XTerm*faceName" = "JetBrains Mono:size=12";
    "XTerm*geometry" = "80x24";

    # Gruvbox Dark theme colors
    "XTerm*background" = "#282828";
    "XTerm*foreground" = "#ebdbb2";
    "XTerm*color0" = "#282828";
    "XTerm*color1" = "#cc241d";
    "XTerm*color2" = "#98971a";
    "XTerm*color3" = "#d79921";
    "XTerm*color4" = "#458588";
    "XTerm*color5" = "#b16286";
    "XTerm*color6" = "#689d6a";
    "XTerm*color7" = "#a89984";
    "XTerm*color8" = "#928374";
    "XTerm*color9" = "#fb4934";
    "XTerm*color10" = "#b8bb26";
    "XTerm*color11" = "#fabd2f";
    "XTerm*color12" = "#83a598";
    "XTerm*color13" = "#d3869b";
    "XTerm*color14" = "#8ec07c";
    "XTerm*color15" = "#ebdbb2";

    # Cursor and selection colors
    "XTerm*cursorColor" = "#ebdbb2";
    "XTerm*highlightColor" = "#504945";
    "XTerm*highlightTextColor" = "#ebdbb2";

    # Scrolling and history
    "XTerm*saveLines" = "4096";
    "XTerm*scrollBar" = "true";
    "XTerm*rightScrollBar" = "true";

    # Better copy/paste support
    "XTerm*selectToClipboard" = "true";
    "XTerm*locale" = "true";
    "XTerm*utf8" = "true";
  };
}
