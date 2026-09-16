from setuptools import Distribution, setup, find_packages
from wheel.bdist_wheel import bdist_wheel


class BinaryDistribution(Distribution):
    def has_ext_modules(self):
        return True


class PlatformWheel(bdist_wheel):
    def get_tag(self):
        # UniFFI uses ctypes: native platform, no CPython ABI dependency.
        _, _, platform = super().get_tag()
        return "py3", "none", platform

setup(
    distclass=BinaryDistribution,
    cmdclass={"bdist_wheel": PlatformWheel},
    packages=find_packages(),
    include_package_data=True,
    package_data={
        "flowsdk": ["*.dylib", "*.so", "*.dll"],
    },
)
