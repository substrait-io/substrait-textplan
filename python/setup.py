#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0

"""Setup script for substrait-textplan Python package."""

import os
import sys
from setuptools import setup
from setuptools_rust import Binding, RustExtension

# Read README if it exists
long_description = ""
readme_path = os.path.join(os.path.dirname(__file__), "README.md")
if os.path.exists(readme_path):
    with open(readme_path, encoding="utf-8") as f:
        long_description = f.read()

setup(
    name="substrait-textplan",
    version="0.1.0",
    author="David Sisson",
    author_email="EpsilonPrime@users.noreply.github.com",
    description="Python wrapper for the Substrait TextPlan library",
    long_description=long_description,
    long_description_content_type="text/markdown",
    url="https://github.com/substrait-io/substrait-textplan",
    py_modules=["substrait_textplan"],
    rust_extensions=[
        RustExtension(
            "substrait_textplan.libsubstrait_textplan",
            path="../Cargo.toml",
            binding=Binding.NoBinding,  # We're building a cdylib, not PyO3
            debug=False,
        )
    ],
    setup_requires=["setuptools-rust>=1.5.2"],
    install_requires=[],
    python_requires=">=3.8",
    classifiers=[
        "Development Status :: 3 - Alpha",
        "Intended Audience :: Developers",
        "Topic :: Software Development :: Libraries",
        "License :: OSI Approved :: Apache Software License",
        "Programming Language :: Python :: 3",
        "Programming Language :: Python :: 3.8",
        "Programming Language :: Python :: 3.9",
        "Programming Language :: Python :: 3.10",
        "Programming Language :: Python :: 3.11",
        "Programming Language :: Python :: 3.12",
    ],
    project_urls={
        "Bug Reports": "https://github.com/substrait-io/substrait-textplan/issues",
        "Source": "https://github.com/substrait-io/substrait-textplan",
    },
    zip_safe=False,  # Required for rust extensions
)
