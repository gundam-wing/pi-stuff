# Pinned nixpkgs fetches crate tarballs from crates.io's API endpoint, which
# returns 403 to Nix's default curl User-Agent. Rewriting to static.crates.io
# avoids both the 403 and the duplicate-registry error from extraRegistries.
final: prev: {
  fetchurl =
    args:
    let
      url = args.url or "";
      staticUrl = builtins.replaceStrings
        [ "https://crates.io/api/v1/crates" ]
        [ "https://static.crates.io/crates" ]
        url;
    in
    if staticUrl != url then
      prev.fetchurl (args // { url = staticUrl; })
    else
      prev.fetchurl args;
}
