"""Build app/data/mets.tsv from the 2024 Adult Compendium of Physical Activities (offline, never at runtime).

Source: Herrmann SD, Willis EA, Ainsworth BE et al., "2024 Adult Compendium of Physical Activities",
J Sport Health Sci 2024;13(1):6-12, hosted at pacompendium.com.
  PDF: https://pacompendium.com/wp-content/uploads/2024/01/2024-adult-compendium_1_2024.pdf
MET values and descriptions are copied unaltered: nothing is rounded, combined or reworded. The
maintainers ask that MET values are not changed and that activities of different MET levels are not
combined. Whether the table may be embedded in Tiffin's public repository has been asked of them and
the reply is pending; the header of the written file says so.

Usage:
  build:    pdftotext -raw -enc UTF-8 <pdf> <txt>
            python -I app/tools/build_mets.py <txt> <pdf> app/data/mets.tsv
  check:    python -I app/tools/build_mets.py --check <tsv>
  release:  python -I app/tools/build_mets.py --check-release <tsv>   (fails unless <tsv> is the stand-in)
  stand-in: python -I app/tools/build_mets.py --stand-in <tsv> <out>

Download the PDF into its own new empty folder outside the repository and pass paths as arguments;
the PDF and its text never enter the repository.

Output, one activity per line, tab separated:
  id  code  heading  met  description  shown
`id` is the Compendium code, with "-2" for a code the PDF uses a second time (four codes repeat with
different activities, so the code alone is not a key). `met` is kept as printed ("8.00" stays "8.00").
`shown` is common (offered before typing), search (found by typing) or hidden (the Inactivity rows,
MET about 1, which log about 0 kcal net).
"""
import datetime
import hashlib
import os
import re
import sys

PDF_URL = "https://pacompendium.com/wp-content/uploads/2024/01/2024-adult-compendium_1_2024.pdf"
STATED = 1114  # activities the maintainers state; the PDF text yields fewer, and the header says so
MIN_ROWS = 1111  # every row the PDF text yields (RESEARCH: 1,091 complete lines + 20 continuation shapes)
COLUMNS = "id\tcode\theading\tmet\tdescription\tshown"
SHOWN = ("common", "search", "hidden")
HIDDEN_HEADING = "Inactivity"
STAND_IN = ("# Stand-in: the cited Compendium rows are left out of public builds until the maintainers reply. "
            "Custom activities still work.")
HEADER_PREFIXES = ("# 2024 Adult Compendium", "# MET values unaltered", "# Source:", "# Retrieved:",
                   "# SHA-256", "# Rows:", "# Permission to embed:")

# A complete line: Heading CODE MET description. Code-first lines follow a heading-only line.
FULL = re.compile(r"^(.+?) (\d{5}) (\d{1,2}\.\d+) (\S.*)$")
NO_DESC = re.compile(r"^(.+?) (\d{5}) (\d{1,2}\.\d+)$")
CODE_FIRST = re.compile(r"^(\d{5}) (\d{1,2}\.\d+)(?: (\S.*))?$")

# Rows that must read exactly like this (id: (met as printed, description)). Every repeated code is here
# (03050, 12355, 15450, 17350, 18202: both uses), and 18290 prints its MET with two decimals.
SPOTS = {
    "01003": ("14.0", "Bicycling, mountain, uphill, vigorous"),
    "01270": ("9.0", "Bicycling, stationary, RPM/Spin bike class"),
    "02050": ("6.0", "Resistance (weight lifting - free weight, nautilus or universal-type), power lifting or body building, vigorous effort (Taylor Code 210)"),
    "02150": ("2.3", "Yoga, Hatha"),
    "03050": ("6.0", "Afro-Cuban salsa (Cuban cha-cha-cha, mambo, Afro rumba, contratiempo son steps, orisha/santo movements)"),
    "03050-2": ("5.5", "Anishinaabe Jingle dancing, brisk pace, other traditional American Indian dancing performed by women, moderate effort"),
    "05043": ("3.0", "Vacuuming, general, moderate effort"),
    "07009": ("1.0", "Lying quietly and watching television"),
    "12020": ("7.5", "Jogging, general, self-selected pace"),
    "12060": ("10.5", "Running, 6.7 mph (9 min/mile)"),
    "12355": ("16.0", "Running, hilly terrain, \u00b1100m change in elevation"),
    "12355-2": ("9.0", "Running downhill, 7.0 to 8.9 mph, -10% to -15%"),
    "15450": ("7.3", "Kickboxing"),
    "15450-2": ("7.0", "Kickball"),
    "15675": ("6.8", "Tennis, general, moderate effort"),
    "17200": ("4.8", "Walking, 3.5 to 3.9 mph, level, brisk, firm surface, walking for exercise"),
    "17350": ("8.0", "Walking, Teabag walk, Monty Python Ministry of Silly Walks"),
    "17350-2": ("3.8", "Walking, treadmill, 3.0 to 3.4 mph (4.8 to 5.5 km/h), 0% grade"),
    "18202": ("5.3", "Scuba diving, general, light effort"),
    "18202-2": ("6.8", "Scuba diving, recreational diver"),
    "18230": ("9.8", "Swimming laps, freestyle, fast, vigorous effort"),
    "18290": ("8.00", "Swimming, crawl, medium speed, ~50 yards/minute, vigorous effort"),
}

# Rows offered before typing (D-15 discretion over which; descriptions stay verbatim).
COMMON = set("""
12020 12028 12029 12030 12045 12050 12060 12070 12080 12090 12100 12110 12120 12145
17151 17152 17160 17165 17170 17190 17200 17220 17110 17080 17082 17012 17131
01009 01011 01014 01015 01016 01017 01030 01040 01050 01200 01214 01218 01224 01228 01232 01270
18230 18240 18250 18255 18260 18270 18280 18300 18310 18100 18224 18220 02120
02050 02052 02054 02056 02058 02020 02022 02030 02032 02035 02040
02150 02153 02155 02160 02175 02185 02101 02103 02105
02210 02214 02000 02005 02006 02068 02310 02048 02049 02060 02064 02070 02071 02072 02073
03014 03030 03031 03033 03040 03042 03070 03078 03090
05010 05021 05030 05032 05035 05043 05049 05050 05060 05070 05092 05100 05120 05130 05041 05175
15055 15610 15675 15690 15255 15265 15710 15660
08095 08160 08245
19160 19090 19252
""".split())
COMMON_RANGE = (100, 140)


def parse(text):
    """Return (rows, strays): rows are dicts code/heading/met/desc, strays are lines that fit no shape."""
    lines = [l.strip() for l in text.replace("\r", "").replace("\x0c", "").split("\n")]
    lines = [l for l in lines if l]
    rows, strays = [], []
    open_row = None  # a row whose description continues on the following lines
    heading = None  # a heading that sat alone on its line, for the code-first row after it
    for i, l in enumerate(lines):
        nxt = lines[i + 1] if i + 1 < len(lines) else ""
        if i == 0 and l.startswith("Major Heading"):
            continue
        if l.startswith("2024 Adult Compendium of Physical Activit"):
            continue  # the page title the PDF text places inside the first page
        m = CODE_FIRST.match(l)
        if m:
            if heading is None:
                strays.append(l)
                continue
            row = {"code": m.group(1), "heading": heading, "met": m.group(2), "desc": m.group(3)}
            rows.append(row)
            open_row = row if row["desc"] is None else None
            heading = None
            continue
        m = FULL.match(l)
        if m:
            rows.append({"code": m.group(2), "heading": m.group(1), "met": m.group(3), "desc": m.group(4)})
            open_row = None
            continue
        m = NO_DESC.match(l)
        if m:
            open_row = {"code": m.group(2), "heading": m.group(1), "met": m.group(3), "desc": None}
            rows.append(open_row)
            continue
        if CODE_FIRST.match(nxt):
            heading, open_row = l, None
            continue
        if open_row is not None:
            open_row["desc"] = l if open_row["desc"] is None else open_row["desc"] + " " + l
            continue
        strays.append(l)
    return rows, strays


def with_ids(rows):
    seen = {}
    for r in rows:
        seen[r["code"]] = seen.get(r["code"], 0) + 1
        n = seen[r["code"]]
        r["id"] = r["code"] if n == 1 else f"{r['code']}-{n}"
    return rows


def shown_of(row):
    if row["heading"] == HIDDEN_HEADING:
        return "hidden"
    return "common" if row["id"] in COMMON else "search"


def not_placed(text, rows):
    """Codes the raw text shows as `CODE MET` that the table lacks (a repeated code counts once per use)."""
    seen = {}
    for l in text.replace("\r", "").replace("\x0c", "").split("\n"):
        for code in re.findall(r"(?<!\d)(\d{5}) \d{1,2}\.\d+(?!\d)", l.strip()):
            seen[code] = seen.get(code, 0) + 1
    placed = {}
    for r in rows:
        placed[r["code"]] = placed.get(r["code"], 0) + 1
    return sorted(c for c, n in seen.items() if n > placed.get(c, 0))


def header_lines(sha, placed, missing):
    return [
        "# 2024 Adult Compendium of Physical Activities, Herrmann SD, Willis EA, Ainsworth BE et al., J Sport Health Sci 2024;13(1):6-12, pacompendium.com.",
        "# MET values unaltered; activities are not combined.",
        f"# Source: {PDF_URL}",
        f"# Retrieved: {datetime.date.today().isoformat()}",
        f"# SHA-256 of the PDF: {sha}",
        f"# Rows: {placed:,} of {STATED:,} stated. Not placed: "
        + (", ".join(missing) + "." if missing else "the remaining codes do not appear in the PDF text."),
        "# Permission to embed: asked of the maintainers, reply pending; must be settled before any public release (STATE.md). "
        "Until then public release snapshots ship the stand-in from build_mets.py --stand-in instead of these rows.",
    ]


def check_text(text):
    """Return (failures, summary). Works on a written file alone, without the PDF."""
    fails = []
    lines = text.replace("\r\n", "\n").split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    header = [l for l in lines if l.startswith("#")]
    body = [l for l in lines if not l.startswith("#")]
    for p in HEADER_PREFIXES:
        if not any(h.startswith(p) for h in header):
            fails.append(f"missing header line starting {p!r}")
    rows_line = next((h for h in header if h.startswith("# Rows:")), "")
    m = re.fullmatch(r"# Rows: ([\d,]+) of 1,114 stated\. Not placed: .+", rows_line)
    if rows_line and not m:
        fails.append(f"malformed {rows_line[:60]!r}")
    if not body or body[0] != COLUMNS:
        fails.append("column line missing or wrong")
        return fails, ""
    rows = body[1:]
    if any(h.startswith("# Stand-in:") for h in header):
        if rows:
            fails.append(f"stand-in must have no rows, found {len(rows)}")
        return fails, "OK: stand-in, 0 rows"
    ids, by_id = set(), {}
    for n, l in enumerate(rows, 1):
        f = l.split("\t")
        if len(f) != 6:
            fails.append(f"row {n}: {len(f)} fields, want 6: {l[:80]!r}")
            continue
        rid, code, heading, met, desc, shown = f
        if rid in ids:
            fails.append(f"row {n}: id {rid} repeated")
        ids.add(rid)
        by_id[rid] = f
        if not re.fullmatch(r"\d{5}(-\d+)?", rid) or rid[:5] != code:
            fails.append(f"row {n}: id {rid!r} does not fit code {code!r}")
        try:
            ok = 1.0 <= float(met) <= 23.0
        except ValueError:
            ok = False
        if not ok:
            fails.append(f"row {n}: id {rid}: MET {met!r} is not a number from 1.0 to 23.0")
        if not desc:
            fails.append(f"row {n}: id {rid}: empty description")
        if shown not in SHOWN:
            fails.append(f"row {n}: id {rid}: shown {shown!r}")
        if heading == HIDDEN_HEADING and shown != "hidden":
            fails.append(f"row {n}: id {rid}: Inactivity row is {shown}, must be hidden")
    if m and int(m.group(1).replace(",", "")) != len(rows):
        fails.append(f"header says {m.group(1)} rows, file has {len(rows)}")
    common = sum(1 for l in rows if l.endswith("\tcommon"))
    if not COMMON_RANGE[0] <= common <= COMMON_RANGE[1]:
        fails.append(f"{common} common rows, want {COMMON_RANGE[0]} to {COMMON_RANGE[1]}")
    if len(rows) < MIN_ROWS:
        fails.append(f"only {len(rows)} rows, want at least {MIN_ROWS}")
    for rid, (met, desc) in SPOTS.items():
        f = by_id.get(rid)
        if f is None:
            fails.append(f"spot check: {rid} missing")
        elif f[3] != met or (desc is not None and f[4] != desc):
            fails.append(f"spot check: {rid} reads {f[3]} {f[4]!r}, want {met} {desc!r}")
    return fails, f"OK: full table, {len(rows)} rows"


def check_file(path):
    with open(path, "rb") as fh:
        return check_text(fh.read().decode("utf-8"))


def build(txt_path, pdf_path, out_path):
    with open(txt_path, "rb") as fh:
        text = fh.read().decode("utf-8")
    rows, strays = parse(text)
    with open(pdf_path, "rb") as fh:
        sha = hashlib.sha256(fh.read()).hexdigest()
    rows = with_ids(rows)
    unknown = sorted(COMMON - {r["id"] for r in rows})
    if unknown:
        sys.exit(f"FAIL: COMMON names ids that are not in the table: {unknown}")
    missing = not_placed(text, rows)
    bad = [r["id"] for r in rows if r["desc"] is None or "\t" in r["desc"] + r["heading"]]
    if bad:
        sys.exit(f"FAIL: rows without a usable description: {bad}")
    print(f"parsed {len(rows):,} of {STATED:,} stated")
    print("not placed: " + (", ".join(missing) if missing else "none can be found in the text"))
    odd = [r["id"] for r in rows if "\ufffd" in r["desc"] + r["heading"]]
    print("rows with a replacement character: " + (", ".join(odd) if odd else "none"))
    print("rows with other non-ASCII text, to compare by eye with the PDF: "
          + ", ".join(r["id"] for r in rows if any(ord(c) > 126 for c in r["desc"] + r["heading"])))
    for s in strays:
        print(f"stray line (not a row): {s!r}")
    out = header_lines(sha, len(rows), missing) + [COLUMNS]
    out += ["\t".join((r["id"], r["code"], r["heading"], r["met"], r["desc"], shown_of(r))) for r in rows]
    tmp = out_path + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(out) + "\n")
    fails, summary = check_file(tmp)
    if fails:
        os.remove(tmp)
        print("\n".join("FAIL: " + f for f in fails))
        sys.exit(1)
    os.replace(tmp, out_path)  # like write_atomic: only a file that passed its own check replaces the table
    print(summary)


def stand_in(tsv_path, out_path):
    """Write the header-only file public release snapshots ship until the maintainers allow embedding."""
    fails, _ = check_file(tsv_path)
    if fails:
        sys.exit("FAIL: the source table does not pass --check:\n" + "\n".join(fails))
    with open(tsv_path, "rb") as fh:
        header = [l for l in fh.read().decode("utf-8").replace("\r\n", "\n").split("\n") if l.startswith("#")]
    with open(out_path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(header + [STAND_IN, COLUMNS]) + "\n")


def main(argv):
    if len(argv) == 2 and argv[0] == "--check":
        fails, summary = check_file(argv[1])
        if fails:
            print("\n".join("FAIL: " + f for f in fails))
            return 1
        print(summary)
        return 0
    if len(argv) == 2 and argv[0] == "--check-release":
        # shortcut: until the Compendium maintainers allow embedding, a public release ships only the stand-in; drop this once they do
        fails, summary = check_file(argv[1])
        if fails or not summary.startswith("OK: stand-in"):
            print("\n".join("FAIL: " + f for f in fails) or "FAIL: the full Compendium table can't ship until permission to embed is settled; use --stand-in")
            return 1
        print(summary)
        return 0
    if len(argv) == 3 and argv[0] == "--stand-in":
        stand_in(argv[1], argv[2])
        return 0
    if len(argv) == 3 and not argv[0].startswith("--"):
        build(*argv)
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
