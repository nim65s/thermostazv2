#!/usr/bin/env python

from enum import IntEnum

from serial import Serial

HEAD = [0xFF, 0xFF]

if __name__ == "__main__":
    with Serial("/dev/ttyACM2") as port:
        port.write([1, 1, 42, 0, 32])
