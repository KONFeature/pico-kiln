"""
Tests for the offline PID-tuning analyser.

Run from the repo root:
    python3 -m unittest discover -s scripts/analyzer/tests -v
"""

import math
import os
import random
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..'))

from analyzer.data import detect_phases
from analyzer.thermal import ThermalModel, fit_thermal_model, H_MAX
from analyzer.pid import calculate_amigo, calculate_cohen_coon, calculate_ziegler_nichols
from analyzer.reporting import assess_test_quality, generate_results_json


def make_model(K, L, T):
    model = ThermalModel()
    model.steady_state_gain = K
    model.dead_time_s = L
    model.time_constant_s = T
    return model


def synthetic_step(K, tau, L, ssr, duration, dt=2.0, noise=0.2, ambient=20.0,
                   idle=20.0, seed=1):
    """Open-loop FOPDT step response with Gaussian sensor noise (idle SSR=0 first)."""
    rng = random.Random(seed)
    time, temp, out = [], [], []
    t = 0.0
    while t <= idle + duration:
        elapsed = t - idle
        ssr_now = ssr if elapsed >= 0 else 0.0
        if elapsed > L:
            y = ambient + K * ssr * (1.0 - math.exp(-(elapsed - L) / tau))
        else:
            y = ambient
        time.append(t)
        temp.append(y + rng.gauss(0.0, noise))
        out.append(ssr_now)
        t += dt
    return {'time': time, 'temp': temp, 'ssr_output': out}


class PidFormulaTests(unittest.TestCase):
    """Literature values for K=4, L=14, T=760 (hand computed)."""
    K, L, T = 4.0, 14.0, 760.0

    def setUp(self):
        self.model = make_model(self.K, self.L, self.T)

    def test_ziegler_nichols(self):
        # Kp = 1.2 T/(K L) = 912/56, Ti = 2L = 28, Td = L/2 = 7
        p = calculate_ziegler_nichols(self.model)
        self.assertAlmostEqual(p.kp, 912.0 / 56.0, places=6)
        self.assertAlmostEqual(p.ki, (912.0 / 56.0) / 28.0, places=6)
        self.assertAlmostEqual(p.kd, (912.0 / 56.0) * 7.0, places=6)

    def test_amigo(self):
        # Kp = (0.2 + 0.45*760/14)/4 = 6.157143; Ti = 14*(5.6+608)/(14+76) = 95.44889
        # Td = 0.5*14*760/(4.2+760) = 6.961529
        p = calculate_amigo(self.model)
        self.assertAlmostEqual(p.kp, 6.157143, places=5)
        self.assertAlmostEqual(p.ki, 6.157143 / 95.44889, places=5)
        self.assertAlmostEqual(p.kd, 6.157143 * 6.961529, places=4)
        # Regression: the old Ti formula made Ki ~17x too small
        self.assertGreater(p.ki, 0.05)

    def test_cohen_coon(self):
        # r = 14/760 = 0.0184211
        # Kp = 0.25*54.28571*(4/3 + r/4) = 18.15773; Ti = 14*(32+6r)/(13+8r) = 34.1930
        # Td = 56/(11+2r) = 5.07404
        p = calculate_cohen_coon(self.model)
        r = 14.0 / 760.0
        kp = 0.25 * (760.0 / 14.0) * (4.0 / 3.0 + r / 4.0)
        ti = 14.0 * (32.0 + 6.0 * r) / (13.0 + 8.0 * r)
        td = 56.0 / (11.0 + 2.0 * r)
        self.assertAlmostEqual(p.kp, kp, places=9)
        self.assertAlmostEqual(kp, 18.1577, places=3)
        self.assertAlmostEqual(ti, 34.1930, places=3)
        self.assertAlmostEqual(td, 5.0740, places=3)
        self.assertAlmostEqual(p.ki, kp / ti, places=9)
        self.assertAlmostEqual(p.kd, kp * td, places=9)


class ThermalFitTests(unittest.TestCase):
    def fit(self, data):
        return fit_thermal_model(data, detect_phases(data))

    def test_recovers_parameters_at_steady_state(self):
        K, tau, L, ssr = 4.0, 300.0, 15.0, 50.0
        data = synthetic_step(K, tau, L, ssr, duration=2400.0)
        model = self.fit(data)
        self.assertAlmostEqual(model.steady_state_gain, K, delta=0.1 * K)
        self.assertAlmostEqual(model.time_constant_s, tau, delta=0.1 * tau)
        self.assertAlmostEqual(model.dead_time_s, L, delta=0.1 * L + 1.0)
        self.assertTrue(model.steady_state_reached)
        self.assertNotEqual(model.gain_confidence, "LOW")
        self.assertLess(model.fit_rmse, 0.5)

    def test_truncated_step_is_flagged_low_confidence(self):
        # tau = 3000 s but the step stops after 600 s: gain/tau are extrapolated
        data = synthetic_step(4.0, 3000.0, 15.0, 50.0, duration=600.0)
        model = self.fit(data)
        self.assertEqual(model.gain_confidence, "LOW")
        self.assertFalse(model.steady_state_reached)
        self.assertTrue(any("steady state" in w for w in model.identification_warnings))
        phases = detect_phases(data)
        self.assertEqual(assess_test_quality(data, phases, model), 'POOR')

    def test_insufficient_data_disables_gain_scheduling(self):
        data = synthetic_step(4.0, 300.0, 15.0, 50.0, duration=2400.0)
        model = self.fit(data)  # a single plateau/step: no >=2 gain points
        self.assertEqual(model.heat_loss_coefficient, 0.0)
        self.assertEqual(model.heat_loss_method, "disabled")
        self.assertFalse(model.gain_scheduling_enabled)
        self.assertIn("disabled", model.heat_loss_note)
        self.assertNotEqual(model.heat_loss_coefficient, 0.0001)

        results = generate_results_json(data, detect_phases(data), model, {}, 'POOR', 'amigo')
        self.assertFalse(results['thermal_model']['gain_scheduling_enabled'])
        self.assertEqual(results['thermal_model']['heat_loss_coefficient'], 0.0)

    def test_heat_loss_is_clamped_to_firmware_range(self):
        from analyzer.thermal import _apply_heat_loss
        model = make_model(10.0, 15.0, 300.0)
        model.ambient_temp = 20.0
        model.gain_confidence = "HIGH"
        # K_eff far below K_base -> h = (10/1 - 1)/ (500-20) ~ 0.019 > 0.01
        model.gain_vs_temp = [{'temp': 500.0, 'gain': 1.0, 'ssr': 50.0},
                              {'temp': 600.0, 'gain': 0.9, 'ssr': 50.0}]
        _apply_heat_loss(model, {'time': [], 'temp': []}, [])
        self.assertEqual(model.heat_loss_coefficient, H_MAX)
        self.assertTrue(any("clamped" in w for w in model.identification_warnings))


if __name__ == '__main__':
    unittest.main()
