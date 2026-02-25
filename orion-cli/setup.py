from setuptools import setup

setup(
    name="orionis-cli",
    version="0.1.0",
    description="Orionis CLI",
    py_modules=["orion"],
    entry_points={
        "console_scripts": [
            "orion=orion:main",
        ],
    },
)
