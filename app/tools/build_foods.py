"""Build app/data/foods.tsv from the official food-composition databases.

Sources (all free, public). Each archive is checked against its SHA-256 before it is extracted,
into its own new empty folder outside the repository:
  F  USDA FoodData Central, Foundation Foods, FoodData_Central_foundation_food_csv_2026-04-30.zip
     sha256 70457ee9d9342f43bda2010318c85f04210c689fdeb9cd2da4c513b0e8dbc655
     (public domain, CC0 1.0)
  S  USDA FoodData Central, SR Legacy, FoodData_Central_sr_legacy_food_csv_2018-04.zip
     sha256 b80817294b8850530aaedf2e515c02593b1824f763a0ff356e5c2081643e6fd0
     (public domain, CC0 1.0)
  C  Health Canada, Canadian Nutrient File 2026, cnf_fcen_all-files-data_2026.zip
     sha256 f5faad8977ee6bbdd9d69c8649077cacd87d8658ad200509a4047db1e29edcdd
     (Open Government Licence - Canada)
  N  USDA FoodData Central, FNDDS 2021-2023, FoodData_Central_survey_food_csv_2024-10-31.zip
     sha256 5ccc25ec2777a8982fbb61378a42f415316173eb11e48c9a8ba4cb19f5a4f29c
     (public domain, CC0 1.0)

Usage:
  python -I build_foods.py <foundation_dir> <sr_legacy_dir> <cnf_dir> <fndds_dir> <out.tsv>

Output, one food per line, tab separated:
  id  source  category  name  kcal  protein  fat  carbs  fiber  sugar  sodium_mg  portions
All nutrient values are per 100 g. A blank cell means the source does not report that value
(unknown); 0 means it reports zero. Only kcal is required (reported, or 4P+9F+4C when all three
macros are known). `portions` is "key=grams;key=grams" where key is "ml" (grams per
millilitre) or a count word such as "each", "medium", "large", "slice", "clove", "scoop"
(grams for one).
"""
import csv
import re
import sys
from collections import defaultdict

ML = {"cup": 236.588, "cups": 236.588, "tbsp": 14.787, "tablespoon": 14.787, "tablespoons": 14.787,
      "tsp": 4.929, "teaspoon": 4.929, "teaspoons": 4.929, "fl oz": 29.574, "quart": 946.35,
      "pint": 473.18, "liter": 1000.0, "litre": 1000.0, "milliliter": 1.0, "ml": 1.0}
SKIP_WEIGHT = {"oz", "lb", "g", "serving", "nlea serving", "portion", "cubic inch", "package",
               "container", "bag", "jar", "can or bottle", "quart", "unit", "item"}
COUNT_WORDS = {"small", "medium", "large", "extra large", "slice", "clove", "piece", "stick",
               "scoop", "leaf", "stalk", "head", "fillet", "breast", "thigh", "drumstick", "wing",
               "link", "strip", "can", "fruit", "ear", "egg", "sprig", "spear", "bunch", "cube",
               "patty", "tortilla", "muffin", "bagel", "roll", "bar", "pepper", "onion", "banana",
               "potato", "carrot", "tomato", "apple", "orange", "lemon", "lime", "avocado"}
SR_SKIP_CATS = {"3", "21", "25", "26", "27"}  # baby foods, fast food, restaurant, branded, QC
CNF_SKIP_GROUPS = {"3", "21"}
# WWEIA categories of baby foods and formula
FNDDS_SKIP_CATS = {"9002", "9004", "9006", "9007", "9008", "9010", "9012", "9202", "9204", "9402", "9404"}
# A source below its minimum means a mapping broke quietly (RESEARCH Pitfall 2): the build stops.
MINIMUMS = {"F": 300, "S": 7000, "C": 5500, "N": 5000}
# Legacy nutrient numbers, used by FNDDS and CNF: energy, protein, fat, carbs, fiber, sugars, sodium
LEGACY = {"208": "kcal", "203": "p", "204": "f", "205": "c", "291": "fib", "269": "sug", "307": "na"}
# The seven value columns, in file order
KEYS = ("kcal", "p", "f", "c", "fib", "sug", "na")
SOURCE_LINES = [
    "# F: USDA FoodData Central, Foundation Foods 2026-04-30, sha256 70457ee9d9342f43bda2010318c85f04210c689fdeb9cd2da4c513b0e8dbc655, public domain (CC0 1.0)",
    "# S: USDA FoodData Central, SR Legacy 2018-04, sha256 b80817294b8850530aaedf2e515c02593b1824f763a0ff356e5c2081643e6fd0, public domain (CC0 1.0)",
    "# C: Health Canada, Canadian Nutrient File 2026, sha256 f5faad8977ee6bbdd9d69c8649077cacd87d8658ad200509a4047db1e29edcdd, Open Government Licence - Canada",
    "# N: USDA FoodData Central, FNDDS 2021-2023 (survey 2024-10-31), sha256 5ccc25ec2777a8982fbb61378a42f415316173eb11e48c9a8ba4cb19f5a4f29c, public domain (CC0 1.0)",
]


def norm_count(word):
    w = word.strip().lower()
    w = re.sub(r"\(.*", "", w).strip()
    w = w.split(",")[0].strip()
    if w.endswith("es") and w[:-2] in COUNT_WORDS:
        w = w[:-2]
    elif w.endswith("s") and w[:-1] in COUNT_WORDS:
        w = w[:-1]
    if w in ("whole", "unit", "item", "nlea serving"):
        return None
    for size in ("extra large", "small", "medium", "large"):
        if w.startswith(size):
            return size
    if w in COUNT_WORDS:
        return w
    return None


class Portions:
    def __init__(self):
        self.ml = []  # grams per ml candidates
        self.count = {}

    @staticmethod
    def volume_rank(rest):
        """Which household volume to trust for grams-per-ml: the plain one ("1 cup", "1 tbsp"),
        then "fluid"/"liquid", then ordinary cut sizes; never "whipped" or "sifted" ones first."""
        if rest == "":
            return 0
        if rest.startswith(("fluid", "liquid", "whole", "raw")):
            return 1
        if any(w in rest for w in ("whipped", "sifted", "unpacked", "spooned", "puffed", "popped")):
            return 9
        return 5

    def add(self, amount, desc, grams):
        if amount <= 0 or grams <= 0:
            return
        d = desc.strip().lower()
        for k, ml in ML.items():
            if d == k or d.startswith(k + ",") or d.startswith(k + " ") or d.startswith(k + "("):
                rest = d[len(k):].strip(" ,")
                self.ml.append((self.volume_rank(rest), grams / (amount * ml)))
                return
        first = d.split(",")[0].strip()
        if first in SKIP_WEIGHT:
            return
        c = norm_count(d)
        if c is None:
            # "cucumber (8-1/4")", "chicken, bone removed": the food's own name is one whole one
            w = re.sub(r"[^a-z ]", " ", first).split()
            if w and len(w[0]) > 2 and w[0] not in ("unit", "item", "serving", "nlea", "cup", "oz", "lb"):
                c = w[0][:-1] if w[0].endswith("s") and not w[0].endswith("ss") else w[0]
        if c and c not in self.count:
            self.count[c] = grams / amount

    def text(self):
        parts = []
        if self.ml:
            # first listed household volume is the plain one ("cup" before "cup, chopped")
            parts.append("ml=%.3f" % min(self.ml, key=lambda x: x[0])[1])
        for k, v in self.count.items():
            parts.append("%s=%.1f" % (k, v))
        if "each" not in self.count:
            parts_of_one = {"slice", "leaf", "clove", "wedge", "stick", "pat", "strip", "sprig", "spear",
                            "stalk", "cube", "scoop", "can", "bar", "link", "head", "bunch", "floret", "floweret",
                            "piece", "tbsp", "tsp", "cup", "jar", "box", "bottle", "package", "packet"}
            pick = next((k for k in ("medium", "fruit", "large", "egg", "small") if k in self.count), None)
            if pick is None:
                pick = next((k for k in self.count if k not in parts_of_one), None)
            if pick is not None:
                parts.append("each=%.1f" % self.count[pick])
        return ";".join(parts)


def read(path, enc="utf-8"):
    with open(path, encoding=enc, newline="") as f:
        yield from csv.DictReader(f)


def clean_counts(name, portions_text):
    """Keep the portions text but drop count keys that are noise ("hot", "bean"): a key stays only when
    it is a known count word, "each", or the food's own head noun (first word before the first comma)."""
    head = (re.split(r"[ ,]", name.strip().lower())[0] if name.strip() else "")
    head = head[:-1] if head.endswith("s") and not head.endswith("ss") else head
    keep = []
    for part in filter(None, portions_text.split(";")):
        k = part.split("=", 1)[0]
        if k == "ml" or k == "each" or k in COUNT_WORDS or (k == head and len(k) >= 3):
            keep.append(part)
    return ";".join(keep)


def make_row(fid, source, cat, name, n, portions_text):
    """One output row, or None when the food has no usable energy. A value the source does not
    report stays None (unknown); only energy is required: reported, else 4P+9F+4C from the macros."""
    kcal = n.get("kcal", n.get("kcal_s", n.get("kcal_g")))
    p, fat = n.get("p"), n.get("f")
    c = n.get("c", n.get("c2"))
    if kcal is None:
        if None in (p, fat, c):
            return None
        kcal = 4 * p + 9 * fat + 4 * c
    vals = [kcal, p, fat, c, n.get("fib", n.get("fib2")), n.get("sug", n.get("sug2")), n.get("na")]
    return [source + fid, source, cat, name] + vals + [portions_text]


def fraction(text):
    """'1', '1.5' or '1/2' as a number, else None."""
    try:
        if "/" in text:
            a, b = text.split("/", 1)
            return float(a) / float(b)
        return float(text)
    except (ValueError, ZeroDivisionError):
        return None


def usda(dir_, only_ids, data_label, skip_cats):
    foods = {}
    for r in read(dir_ + "/food.csv"):
        if only_ids is not None and r["fdc_id"] not in only_ids:
            continue
        if r["food_category_id"] in skip_cats:
            continue
        foods[r["fdc_id"]] = {"name": r["description"], "cat": r["food_category_id"], "n": {}}
    want = {"2048": "kcal_s", "2047": "kcal_g", "1008": "kcal", "1003": "p", "1004": "f", "1085": "f2",
            "1005": "c", "1050": "c2", "1079": "fib", "2033": "fib2", "2000": "sug", "1063": "sug2", "1093": "na"}
    for r in read(dir_ + "/food_nutrient.csv"):
        f = foods.get(r["fdc_id"])
        k = want.get(r["nutrient_id"])
        if f is None or k is None or r["amount"] == "":
            continue
        f["n"][k] = float(r["amount"])
    units = {r["id"]: r["name"] for r in read(dir_ + "/measure_unit.csv")}
    portions = defaultdict(Portions)
    for r in read(dir_ + "/food_portion.csv"):
        if r["fdc_id"] not in foods:
            continue
        try:
            amount = float(r["amount"] or 1)
            grams = float(r["gram_weight"])
        except ValueError:
            continue
        unit = units.get(r["measure_unit_id"], "undetermined")
        desc = r["modifier"] if unit == "undetermined" else (unit + ("," + r["modifier"] if r["modifier"] else ""))
        if unit != "undetermined" and r["modifier"] and unit not in ML:
            # e.g. unit "fruit" + modifier "medium (2-2/3" dia)" -> "medium"
            c = norm_count(r["modifier"])
            if c:
                desc = c
        portions[r["fdc_id"]].add(amount, desc, grams)
    out = []
    for fid, f in foods.items():
        n = f["n"]
        if "f" not in n and "f2" in n:
            n["f"] = n["f2"]
        row = make_row(fid, data_label, f["cat"], f["name"], n, portions[fid].text() if fid in portions else "")
        if row:
            out.append(row)
    return out


def fndds(dir_):
    """FNDDS mixed dishes. food_nutrient.nutrient_id holds the legacy nutrient number here (208,
    203...), not the FDC id the other two USDA files use."""
    foods = {}
    for r in read(dir_ + "/food.csv"):
        if r["food_category_id"] in FNDDS_SKIP_CATS:
            continue
        foods[r["fdc_id"]] = {"name": r["description"], "cat": r["food_category_id"], "n": {}}
    for r in read(dir_ + "/food_nutrient.csv"):
        f = foods.get(r["fdc_id"])
        k = LEGACY.get(r["nutrient_id"])
        if f is None or k is None or r["amount"] == "":
            continue
        f["n"][k] = float(r["amount"])
    portions = defaultdict(Portions)
    for r in read(dir_ + "/food_portion.csv"):
        if r["fdc_id"] not in foods:
            continue
        desc = r["portion_description"].strip()
        low = desc.lower()
        if "quantity not specified" in low or "guideline amount" in low or "cubic inch" in low or "surface inch" in low:
            continue
        try:
            grams = float(r["gram_weight"])
        except ValueError:
            continue
        m = re.match(r"^([\d./]+)\s+(.+)$", desc)
        amount = fraction(m.group(1)) if m else None
        if grams <= 0 or not amount:
            continue
        portions[r["fdc_id"]].add(amount, m.group(2), grams)
    out = []
    for fid, f in foods.items():
        row = make_row(fid, "N", f["cat"], f["name"], f["n"], portions[fid].text() if fid in portions else "")
        if row:
            out.append(row)
    return out


def cnf(dir_):
    """Canadian Nutrient File 2026: UTF-8 with a BOM, new file and column names, and portions
    already in grams per measure (the 2015 file gave multiples of 100 g)."""
    enc = "utf-8-sig"
    foods = {}
    for r in read(dir_ + "/Food_Name.csv", enc):
        if r["CNF_Food_Group_Code"] in CNF_SKIP_GROUPS:
            continue
        foods[r["Food_Code"]] = {"name": r["Food_Description_EN"], "cat": "cnf" + r["CNF_Food_Group_Code"], "n": {}}
    for r in read(dir_ + "/Nutrient_Amount.csv", enc):
        f = foods.get(r["Food_Code"])
        k = LEGACY.get(r["Nutrient_Code"])
        if f is None or k is None:
            continue
        try:
            f["n"][k] = float(r["Nutrient_Amount"])
        except ValueError:
            pass
    measures = {r["Measure_Code"]: r["Measure_Description_and_Unit_EN"] for r in read(dir_ + "/Measure_Name.csv", enc)}
    portions = defaultdict(Portions)
    for r in read(dir_ + "/Measure_Weight_Conversion.csv", enc):
        if r["Food_Code"] not in foods or r["Measure_Type_Code"] != "6":  # 6 = user-defined; 3 refuse, 9 yield
            continue
        desc = measures.get(r["Measure_Code"], "").strip().lower()
        try:
            grams = float(r["Measure_Weight_Conversion"])  # grams per measure
        except ValueError:
            continue
        m = re.match(r"^([\d.]+)\s*ml$", desc)  # only a bare "N ml" is a density row
        if m:
            if grams > 0 and float(m.group(1)) > 0:
                portions[r["Food_Code"]].ml.append((0, grams / float(m.group(1))))
            continue
        m = re.match(r"^(\d+(?:\.\d+)?|1/2|1/4)\s+(.*)$", desc)
        if m:
            portions[r["Food_Code"]].add(fraction(m.group(1)), m.group(2), grams)
    out = []
    for fid, f in foods.items():
        row = make_row(fid, "C", f["cat"], f["name"], f["n"], portions[fid].text() if fid in portions else "")
        if row:
            out.append(row)
    return out


def main():
    fnd, sr, cnf_dir, fndds_dir, out_path = sys.argv[1:6]
    foundation_ids = {r["fdc_id"] for r in read(fnd + "/foundation_food.csv")}
    rows = usda(fnd, foundation_ids, "F", set())
    rows += usda(sr, None, "S", SR_SKIP_CATS)
    rows += cnf(cnf_dir)
    rows += fndds(fndds_dir)
    counts = {s: sum(1 for r in rows if r[1] == s) for s in MINIMUMS}
    blank = {s: sum(1 for r in rows if r[1] == s and any(v is None for v in r[5:8])) for s in MINIMUMS}
    for s in MINIMUMS:
        print("source %s: %d rows, %d kept with a blank protein, fat or carbs" % (s, counts[s], blank[s]), file=sys.stderr)
    low = ["%s has %d rows, minimum %d" % (s, counts[s], m) for s, m in MINIMUMS.items() if counts[s] < m]
    if low:
        sys.exit("Build stopped, a source fell below its minimum: " + "; ".join(low))
    # Foundation entries often list no household portions: borrow them from the same food in
    # SR Legacy / CNF (never from FNDDS dishes, and FNDDS rows borrow nothing).
    # Negative lab values (e.g. carbs by difference of -0.4) become 0.
    by_name = {}
    for r in rows:
        if r[1] == "N":
            continue
        for part in filter(None, r[11].split(";")):
            k, v = part.split("=", 1)
            by_name.setdefault(r[3].lower(), {}).setdefault(k, v)
    fixed = []
    for r in rows:
        r = list(r)
        mine = dict(part.split("=", 1) for part in filter(None, r[11].split(";")))
        if r[1] != "N":
            for k, v in by_name.get(r[3].lower(), {}).items():
                mine.setdefault(k, v)
        r[11] = clean_counts(r[3], ";".join("%s=%s" % kv for kv in mine.items()))
        r[4:11] = [None if v is None else max(0.0, v) for v in r[4:11]]
        fixed.append(r)
    rows = fixed
    # Checks that list instead of silently accepting (alcohol, polyols and fibre explain some kcal gaps).
    off = []
    for r in rows:
        kcal, p, fat, c = r[4:8]
        if None in (kcal, p, fat, c):
            continue
        calc = 4 * p + 4 * c + 9 * fat
        if abs(kcal - calc) > 0.15 * calc and abs(kcal - calc) > 5:
            off.append(r[3])
    zero = sum(1 for r in rows if not r[4])
    print("%d rows with kcal more than 15%% from 4P+4C+9F; first 20: %s" % (len(off), off[:20]), file=sys.stderr)
    print("%d rows with zero energy (%.2f%% of %d)" % (zero, 100.0 * zero / len(rows), len(rows)), file=sys.stderr)
    if zero > 0.01 * len(rows):
        sys.exit("Build stopped: more than 1%% of rows have zero energy (%d of %d)" % (zero, len(rows)))
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write("# Food composition per 100 g; a blank value is not reported (unknown), 0 is reported as zero. Built by tools/build_foods.py\n")
        for line in SOURCE_LINES:
            f.write(line + "\n")
        for r in rows:
            name = r[3].replace("\t", " ").replace("\n", " ")
            f.write("\t".join([r[0], r[1], r[2], name] + ["" if v is None else "%g" % round(v, 2) for v in r[4:11]] + [r[11]]) + "\n")
    print(len(rows), "foods written")


if __name__ == "__main__":
    main()
