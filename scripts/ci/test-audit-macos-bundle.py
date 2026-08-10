#!/usr/bin/env python3
"""Exercise scripts/ci/audit-macos-bundle.sh on Linux with stub otool/vtool.

The audit script only ever shells out to `otool -L`, `vtool -show-build`, find,
grep and sort. Stubbing the two Mach-O tools lets the control flow -- the
worklist walk, the @rpath resolution, the cycle guard, the min-OS assert -- be
tested here instead of discovering a bash 3.2 mistake in CI.
"""
import os
import shutil
import subprocess
import sys

ROOT = os.environ.get("AUDIT_TEST_DIR", "/tmp/speakoflow-audit-test")
AUDIT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "audit-macos-bundle.sh")

OTOOL = """#!/usr/bin/env bash
# stub: otool -L <file>
f="${2:-}"
echo "$f:"
b="$(basename "$f")"
d="%s/deps/$b"
if [ -f "$d" ]; then
  while IFS= read -r dep; do
    [ -n "$dep" ] && printf '\\t%%s (compatibility version 1.0.0, current version 1.0.0)\\n' "$dep"
  done < "$d"
fi
""" % ROOT

VTOOL = """#!/usr/bin/env bash
# stub: vtool -show-build <file>
f="${3:-}"
b="$(basename "$f")"
v="$(cat "%s/minos/$b" 2>/dev/null || echo 13.0)"
echo "Load command 8"
echo "      cmd LC_BUILD_VERSION"
echo "    minos $v"
""" % ROOT


def setup_stubs():
    binpath = os.path.join(ROOT, "bin")
    os.makedirs(binpath, exist_ok=True)
    for name, body in (("otool", OTOOL), ("vtool", VTOOL)):
        p = os.path.join(binpath, name)
        with open(p, "w") as fh:
            fh.write(body)
        os.chmod(p, 0o755)
    return binpath


def make_case(name, exe_deps, lib_deps, minos=None):
    """Build a fake .app. lib_deps maps dylib filename -> list of load paths."""
    app = os.path.join(ROOT, name + ".app")
    shutil.rmtree(app, ignore_errors=True)
    macos = os.path.join(app, "Contents", "MacOS")
    fw = os.path.join(app, "Contents", "Frameworks")
    os.makedirs(macos)
    os.makedirs(fw)

    exe = os.path.join(macos, "speakoflow")
    with open(exe, "w") as fh:
        fh.write("#\n")
    os.chmod(exe, 0o755)

    deps_dir = os.path.join(ROOT, "deps")
    minos_dir = os.path.join(ROOT, "minos")
    shutil.rmtree(deps_dir, ignore_errors=True)
    shutil.rmtree(minos_dir, ignore_errors=True)
    os.makedirs(deps_dir)
    os.makedirs(minos_dir)

    with open(os.path.join(deps_dir, "speakoflow"), "w") as fh:
        fh.write("\n".join(exe_deps) + "\n")

    for lib, deps in lib_deps.items():
        open(os.path.join(fw, lib), "w").close()
        with open(os.path.join(deps_dir, lib), "w") as fh:
            fh.write("\n".join(deps) + "\n")

    for fname, v in (minos or {}).items():
        with open(os.path.join(minos_dir, fname), "w") as fh:
            fh.write(v + "\n")

    return app


def run(app, declared=None, timeout=60):
    env = dict(os.environ)
    env["PATH"] = os.path.join(ROOT, "bin") + ":" + env["PATH"]
    cmd = ["bash", AUDIT, app]
    if declared:
        cmd.append(declared)
    try:
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=timeout)
        return r.returncode, r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        return "TIMEOUT", "(script did not terminate -- cycle guard failed)"


def check(label, expect_pass, rc, out):
    ok = (rc == 0) if expect_pass else (rc not in (0, "TIMEOUT"))
    print("%s  %s  (exit %s)" % ("PASS" if ok else "FAIL", label, rc))
    if not ok:
        print("    ---- output ----")
        for line in out.strip().splitlines():
            print("    " + line)
    return ok


def main():
    setup_stubs()
    results = []

    # 1. The real issue #19 shape: executable is clean, the bundled dylib keeps
    #    absolute Homebrew paths. The OLD audit passed this. This must fail.
    app = make_case(
        "Broken",
        ["@rpath/libonnxruntime.dylib", "/usr/lib/libSystem.B.dylib"],
        {"libonnxruntime.dylib": [
            "@rpath/libonnxruntime.dylib",
            "/usr/local/opt/onnx/lib/libonnx.dylib",
            "/usr/local/opt/abseil/lib/libabsl_base.2601.0.0.dylib",
            "/usr/lib/libc++.1.dylib",
        ]},
    )
    rc, out = run(app, "14.0")
    results.append(check("issue #19 shape is rejected", False, rc, out))
    if "/usr/local/opt/onnx/lib/libonnx.dylib" not in out:
        print("FAIL  error message does not name the offending library")
        results.append(False)
    else:
        print("PASS  error names the offending library")
        results.append(True)

    # 2. Properly bundled: every dependency present in Frameworks, cross-referenced
    #    the way dylibbundler leaves them. Must pass.
    app = make_case(
        "Fixed",
        ["@rpath/libonnxruntime.dylib", "/usr/lib/libSystem.B.dylib"],
        {
            "libonnxruntime.dylib": [
                "@rpath/libonnxruntime.dylib",
                "@rpath/libonnx.dylib",
                "@rpath/libabsl_base.dylib",
                "/usr/lib/libc++.1.dylib",
            ],
            "libonnx.dylib": ["@rpath/libonnx.dylib", "@rpath/libabsl_base.dylib"],
            "libabsl_base.dylib": ["@rpath/libabsl_base.dylib", "/usr/lib/libSystem.B.dylib"],
        },
        minos={"speakoflow": "13.0", "libonnxruntime.dylib": "14.0",
               "libonnx.dylib": "14.0", "libabsl_base.dylib": "14.0"},
    )
    rc, out = run(app, "14.0")
    results.append(check("correctly bundled app is accepted", True, rc, out))

    # 3. Mutual references, which is exactly how abseil's libraries look. The walk
    #    must terminate rather than loop forever.
    app = make_case(
        "Cycle",
        ["@rpath/libA.dylib"],
        {
            "libA.dylib": ["@rpath/libA.dylib", "@rpath/libB.dylib"],
            "libB.dylib": ["@rpath/libB.dylib", "@rpath/libA.dylib"],
        },
        minos={"speakoflow": "13.0", "libA.dylib": "13.0", "libB.dylib": "13.0"},
    )
    rc, out = run(app, "14.0", timeout=30)
    results.append(check("mutually-referencing dylibs terminate", True, rc, out))

    # 4. A dependency that is referenced but was never copied in.
    app = make_case(
        "Missing",
        ["@rpath/libonnxruntime.dylib"],
        {"libonnxruntime.dylib": ["@rpath/libonnxruntime.dylib", "@rpath/libghost.dylib"]},
    )
    rc, out = run(app, "14.0")
    results.append(check("missing bundled dependency is rejected", False, rc, out))

    # 5. The minimumSystemVersion lie: binaries need 14.0, bundle claims 10.15.
    app = make_case(
        "MinOsLie",
        ["@rpath/libonnxruntime.dylib"],
        {"libonnxruntime.dylib": ["@rpath/libonnxruntime.dylib"]},
        minos={"speakoflow": "10.15", "libonnxruntime.dylib": "14.0"},
    )
    rc, out = run(app, "10.15")
    results.append(check("understated minimumSystemVersion is rejected", False, rc, out))

    # 6. Same bundle, honest declaration. Must pass.
    rc, out = run(app, "14.0")
    results.append(check("honest minimumSystemVersion is accepted", True, rc, out))

    print("")
    print("%d/%d checks behaved correctly" % (sum(1 for r in results if r), len(results)))
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
