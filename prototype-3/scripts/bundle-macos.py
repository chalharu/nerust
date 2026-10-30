#!/usr/bin/env python3
"""Build, ad-hoc sign and optionally verify a relocatable AppKit prototype."""
import argparse
import json
import os
import pathlib
import plistlib
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", choices=["wakui3-settings", "wakui3-library"], default="wakui3-settings")
    parser.add_argument("--profile", choices=["dev", "release"], default="dev")
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent.parent
    result = subprocess.run(["cargo", "build", "--locked", "-p", args.package, "--profile", args.profile,
                             "--no-default-features", "--features", "backend-macos", "--message-format=json"],
                            cwd=root, stdout=subprocess.PIPE, text=True, check=True)
    messages = [json.loads(line) for line in result.stdout.splitlines()]
    executable = next(pathlib.Path(item["executable"]) for item in messages
                      if item.get("executable") and item.get("target", {}).get("name") == args.package)
    native = next(pathlib.Path(item["out_dir"]) / "libWakui3AppKit.dylib" for item in messages
                  if item.get("reason") == "build-script-executed" and "wakui3-macos" in item["package_id"])
    with tempfile.TemporaryDirectory(prefix="wakui3-bundle-", dir=root / "target") as temporary:
        bundle = pathlib.Path(temporary) / f"{args.package}.app"
        macos = bundle / "Contents/MacOS"
        frameworks = bundle / "Contents/Frameworks"
        macos.mkdir(parents=True)
        frameworks.mkdir()
        binary = macos / args.package
        shutil.copy2(executable, binary)
        shutil.copy2(native, frameworks / native.name)
        subprocess.run(["install_name_tool", "-add_rpath", "@executable_path/../Frameworks", str(binary)], check=True)
        commands = subprocess.check_output(["otool", "-l", str(binary)], text=True).splitlines()
        in_rpath = False
        for line in commands:
            if line.strip().startswith("cmd "):
                in_rpath = line.strip() == "cmd LC_RPATH"
            if in_rpath and line.strip().startswith("path "):
                value = line.strip()[5:].rsplit(" (offset", 1)[0]
                if pathlib.Path(value).is_absolute():
                    subprocess.run(["install_name_tool", "-delete_rpath", value, str(binary)], check=True)
        with (bundle / "Contents/Info.plist").open("wb") as output:
            plistlib.dump({"CFBundleExecutable": args.package, "CFBundleIdentifier": f"com.wakui.prototype.{args.package}",
                          "CFBundleName": args.package, "CFBundlePackageType": "APPL", "CFBundleVersion": "1",
                          "LSMinimumSystemVersion": os.environ.get("MACOSX_DEPLOYMENT_TARGET", "11.0")}, output)
        subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(bundle)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
        if args.verify:
            with tempfile.TemporaryDirectory(prefix="wakui3-relocated-") as relocated:
                destination = pathlib.Path(relocated) / bundle.name
                shutil.copytree(bundle, destination)
                environment = {key: value for key, value in os.environ.items() if not key.startswith("DYLD_")}
                subprocess.run([str(destination / "Contents/MacOS" / args.package), "--smoke"],
                               cwd=relocated, env=environment, timeout=int(os.environ.get("WAKUI_NATIVE_TIMEOUT_SECONDS", "45")), check=True)
                print("BUNDLE3 PASS relocated signed AppKit smoke without DYLD overrides")
        destination = root / "target/bundle" / bundle.name
        destination.parent.mkdir(exist_ok=True)
        if destination.exists():
            shutil.rmtree(destination)
        shutil.copytree(bundle, destination)
        print(destination)


if __name__ == "__main__":
    main()