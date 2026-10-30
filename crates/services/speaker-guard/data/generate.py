import plistlib
import struct
import sys
from pathlib import Path

MODELS = {
    "j473": {
        "aid": "AID12",
        "card": "AppleJ473",
        "sense_device": 2,
        "sense_channels": 2,
        "speakers": [("", 0, 1)],
    },
}

ATSP_SPEAKER_A = {
    "sensitivity": 1,
    "dc_resistance": 2,
    "coil_resistance": 3,
    "coil_time_constant": 4,
    "magnet_resistance": 5,
    "magnet_time_constant": 6,
    "ambient": 7,
    "limit": 8,
    "attack_ms": 9,
    "release_ms": 10,
    "headroom": 11,
}

ATSP_MAX_REDUCTION_DB = 20.0
CURRENT_FULL_SCALE = 3.75
VOLTAGE_FULL_SCALE = 14.0
PERIOD_FRAMES = 1024


def preset_values(data):
    values = {}
    while len(data) >= 12:
        _, _, count = struct.unpack(">III", data[:12])
        data = data[12:]
        for index in range(count):
            key, value = struct.unpack(">If", data[8 * index : 8 * index + 8])
            values[key] = value
        data = data[8 * count :]
    return values


def speaker_strip(tunings, aid):
    database = plistlib.loads((tunings / "DeviceDSPDatabase.plist").read_bytes())
    general = database[aid]["Built-in Speaker"]["48k - General"]
    paths = [hardware["Path"] for output in general["Output"] for hardware in output["Hardware"]]
    strip = next(path for path in paths if path.endswith(".austrip"))
    return plistlib.loads((tunings / strip).read_bytes())


def atsp(strip):
    for chain in strip["strips"]:
        for effect in chain["effects"]:
            if effect["unit"]["subtype"].to_bytes(4, "big") == b"atsp" and not effect["bypass"]:
                return preset_values(effect["aupreset"]["data"])
    raise SystemExit("no active atsp box in the speaker strip")


def render(model, values):
    a = {name: values[key] for name, key in ATSP_SPEAKER_A.items()}
    lines = [
        f'card "{model["card"]}"',
        f"sense-device {model['sense_device']}",
        f"sense-channels {model['sense_channels']}",
        f"period-frames {PERIOD_FRAMES}",
        f"amplifier-sensitivity {a['sensitivity']:.2f}",
        f"current-full-scale {CURRENT_FULL_SCALE}",
        f"voltage-full-scale {VOLTAGE_FULL_SCALE}",
    ]
    for prefix, current, voltage in model["speakers"]:
        lines += [
            "",
            f'speaker "{prefix}" {{',
            f"    current-channel {current}",
            f"    voltage-channel {voltage}",
            f"    dc-resistance {a['dc_resistance']:.1f}",
            f"    ambient {a['ambient']:.1f}",
            f"    coil-thermal-resistance {a['coil_resistance']:.1f}",
            f"    coil-time-constant {a['coil_time_constant']:.1f}",
            f"    magnet-thermal-resistance {a['magnet_resistance']:.1f}",
            f"    magnet-time-constant {a['magnet_time_constant']:.1f}",
            f"    temperature-limit {a['limit']:.1f}",
            f"    hard-limit-headroom {a['headroom']:.1f}",
            f"    attack {a['attack_ms'] / 1000:.1f}",
            f"    release {a['release_ms'] / 1000:.1f}",
            f"    max-reduction {ATSP_MAX_REDUCTION_DB:.1f}",
            "}",
        ]
    return "\n".join(lines) + "\n"


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: generate.py <macOS root>/System/Library/Audio/Tunings <output dir>")
    tunings = Path(sys.argv[1])
    output = Path(sys.argv[2])
    for name, model in MODELS.items():
        values = atsp(speaker_strip(tunings, model["aid"]))
        (output / f"{name}.kdl").write_text(render(model, values))


main()
