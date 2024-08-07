# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.

from typing import Generic, TypeVar

from materialize.feature_benchmark.measurement import MeasurementType
from materialize.feature_benchmark.scenario_version import ScenarioVersion
from materialize.terminal import (
    COLOR_BAD,
    COLOR_GOOD,
    with_conditional_formatting,
)

T = TypeVar("T")


class Comparator(Generic[T]):
    def __init__(self, type: MeasurementType, name: str, threshold: float) -> None:
        self.name = name
        self.type = type
        self.threshold = threshold
        self._points: list[T] = []
        self.version: ScenarioVersion | None = None

    def append(self, point: T) -> None:
        self._points.append(point)

    def this(self) -> T:
        return self._points[0]

    def this_as_str(self) -> str:
        if self.this() is None:
            return "           None"
        else:
            return f"{self.this():>11.3f}"

    def other(self) -> T:
        return self._points[1]

    def other_as_str(self) -> str:
        if self.other() is None:
            return "           None"
        else:
            return f"{self.other():>11.3f}"

    def set_scenario_version(self, version: ScenarioVersion):
        self.version = version

    def get_scenario_version(self) -> ScenarioVersion:
        assert self.version is not None
        return self.version

    def is_regression(self, threshold: float | None = None) -> bool:
        assert False

    def is_strong_regression(self) -> bool:
        return self.is_regression(threshold=self.threshold * 2)

    def ratio(self) -> float | None:
        assert False

    def human_readable(self, use_colors: bool) -> str:
        return str(self)


class RelativeThresholdComparator(Comparator[float | None]):
    def ratio(self) -> float | None:
        if self._points[0] is None or self._points[1] is None:
            return None
        else:
            return self._points[0] / self._points[1]

    def is_regression(self, threshold: float | None = None) -> bool:
        if threshold is None:
            threshold = self.threshold

        ratio = self.ratio()

        if ratio is None:
            return False
        if ratio > 1:
            return ratio - 1 > threshold
        else:
            return False

    def human_readable(self, use_colors: bool) -> str:
        ratio = self.ratio()
        if ratio is None:
            return "N/A"
        if ratio >= 2:
            return with_conditional_formatting(
                f"{ratio:3.1f} TIMES more/slower", COLOR_BAD, condition=use_colors
            )
        elif ratio > 1:
            return with_conditional_formatting(
                f"{-(1-ratio)*100:3.1f} pct   more/slower",
                COLOR_BAD,
                condition=use_colors,
            )
        elif ratio == 1:
            return "          same"
        elif ratio > 0.5:
            return with_conditional_formatting(
                f"{(1-ratio)*100:3.1f} pct   less/faster",
                COLOR_GOOD,
                condition=use_colors,
            )
        else:
            return with_conditional_formatting(
                f"{(1/ratio):3.1f} times less/faster", COLOR_GOOD, condition=use_colors
            )
