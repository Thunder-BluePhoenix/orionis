from setuptools import setup, find_packages

setup(
    name="orionis-agent",
    version="0.1.0",
    description="Orionis Python runtime tracing agent",
    long_description=open("README.md").read() if __import__("os").path.exists("README.md") else "",
    packages=find_packages(),
    python_requires=">=3.9",
    install_requires=[],   # zero dependencies — stdlib only
    entry_points={
        # Auto-discovered by pytest — no conftest.py needed.
        "pytest11": ["orionis = orionis.pytest_plugin"],
    },
    classifiers=[
        "Programming Language :: Python :: 3",
        "License :: OSI Approved :: Apache Software License",
        "Topic :: Software Development :: Debuggers",
    ],
)
