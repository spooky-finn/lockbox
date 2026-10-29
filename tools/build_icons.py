#!/usr/bin/env python3
"""Builds assets/icons.jsonl from an unpacked simple-icons npm package.

    curl -fsSO https://registry.npmjs.org/simple-icons/-/simple-icons-<version>.tgz
    tar -xzf simple-icons-<version>.tgz          # unpacks into ./package
    python3 -I tools/build_icons.py package

Every icon becomes one JSON line: slug, title, brand colour, SVG path and the
domains it is shown for. Domains come from the icon's `source` URL when that
site is clearly the brand's own, and from assets/icon_domains.txt.
lockbox also matches a domain whose name equals the slug (gitlab.com → gitlab)
at runtime, so those need no entry here.
"""

import json
import re
import shutil
import sys
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
SVG = re.compile(
    r'^<svg role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">'
    r'<title>.*?</title><path d="([^"]+)"/></svg>\s*$',
    re.S,
)
# Suffixes under which names are registered one level deeper (example.co.uk).
TWO_LEVEL = {
    "co.uk", "org.uk", "ac.uk", "com.au", "net.au", "co.jp", "ne.jp",
    "com.br", "com.cn", "com.tr", "co.kr", "co.in", "com.mx", "co.za",
    "com.ru", "net.ru", "org.ru", "msk.ru", "spb.ru", "com.ua", "co.nz",
}


def registrable(host: str) -> str:
    labels = host.lower().strip(".").split(".")
    keep = 3 if ".".join(labels[-2:]) in TWO_LEVEL else 2
    return ".".join(labels[-keep:])


def normalized(text: str) -> str:
    return re.sub(r"[^a-z0-9]", "", text.lower())


def source_domain(icon: dict) -> str | None:
    """The source site's domain, if its name is the brand's name."""
    host = urlsplit(icon["source"]).hostname
    if not host:
        return None
    domain = registrable(host)
    name = domain.split(".")[0]
    if name in (icon["slug"], normalized(icon["title"])):
        return domain
    return None


def manual_domains() -> dict[str, list[str]]:
    table: dict[str, list[str]] = {}
    for number, line in enumerate((ASSETS / "icon_domains.txt").read_text().splitlines(), 1):
        line = line.split("#", 1)[0].split()
        if not line:
            continue
        slug, *domains = line
        if not domains:
            sys.exit(f"icon_domains.txt:{number}: no domains for {slug}")
        table.setdefault(slug, []).extend(d.lower() for d in domains)
    return table


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    package = Path(sys.argv[1])
    version = json.loads((package / "package.json").read_text())["version"]
    icons = json.loads((package / "data" / "simple-icons.json").read_text())
    manual = manual_domains()

    slugs = {icon["slug"] for icon in icons}
    unknown = sorted(set(manual) - slugs)
    if unknown:
        sys.exit(f"icon_domains.txt names unknown icons: {', '.join(unknown)}")

    # The manual table always wins; a source domain goes to the first icon
    # (in slug order) that claims it, so brand variants (uniqlo, uniqlo_ja) share nothing.
    owner: dict[str, str] = {}
    for slug, domains in manual.items():
        for domain in domains:
            if owner.setdefault(domain, slug) != slug:
                sys.exit(f"icon_domains.txt gives {domain} to both {owner[domain]} and {slug}")
    lines = []
    for icon in sorted(icons, key=lambda icon: icon["slug"]):
        svg = (package / "icons" / f"{icon['slug']}.svg").read_text(encoding="utf-8")
        match = SVG.match(svg)
        if not match:
            sys.exit(f"{icon['slug']}.svg is not a single-path 24×24 icon")
        if not re.fullmatch(r"[0-9A-Fa-f]{6}", icon["hex"]):
            sys.exit(f"{icon['slug']}: bad colour {icon['hex']}")
        domains = list(manual.get(icon["slug"], []))
        domain = source_domain(icon)
        if domain and owner.setdefault(domain, icon["slug"]) == icon["slug"] and domain not in domains:
            domains.append(domain)
        lines.append(json.dumps(
            {
                "slug": icon["slug"],
                "title": icon["title"],
                "hex": icon["hex"].upper(),
                "path": match.group(1),
                "domains": domains,
            },
            ensure_ascii=False,
            separators=(",", ":"),
        ))

    (ASSETS / "icons.jsonl").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (ASSETS / "SIMPLE_ICONS_VERSION").write_text(version + "\n")
    shutil.copyfile(package / "LICENSE.md", ASSETS / "LICENSE-simple-icons.md")
    shutil.copyfile(package / "DISCLAIMER.md", ASSETS / "DISCLAIMER-simple-icons.md")
    print(f"simple-icons {version}: {len(lines)} icons, {len(owner)} domains")


if __name__ == "__main__":
    main()
