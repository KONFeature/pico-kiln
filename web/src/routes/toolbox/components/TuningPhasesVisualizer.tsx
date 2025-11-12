import { useState, useMemo } from 'react';
import {
  LineChart,
  Line,
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
  ReferenceArea,
} from 'recharts';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { AlertCircle } from 'lucide-react';
import { FileSourceSelector } from './FileSourceSelector';
import { parseLogCSV, secondsToMinutes, type LogDataPoint } from '@/lib/csv-parser';

type PhaseType = 'heating' | 'cooling' | 'plateau';

interface Phase {
  startIdx: number;
  endIdx: number;
  type: PhaseType;
  avgSsr: number;
  tempStart: number;
  tempEnd: number;
  stepName?: string;
}

interface ChartDataPoint {
  time_minutes: number;
  temp: number;
  ssr_output: number;
  phase?: string;
}

/**
 * Detect tuning phases using physics-based detection:
 * - COOLING: SSR < 5% (natural cooling)
 * - HEATING: SSR ≥ 5% AND temperature rising (>0.5°C/min)
 * - PLATEAU: SSR ≥ 5% AND temperature stable (±0.5°C/min)
 */
function detectPhases(data: LogDataPoint[]): Phase[] {
  const phases: Phase[] = [];
  const SSR_THRESHOLD = 5; // SSR threshold for heating vs cooling
  const RATE_THRESHOLD = 0.5; // °C/min threshold for stable vs rising
  const WINDOW_SIZE = 10; // Number of points to average for rate calculation

  let currentPhase: Phase | null = null;

  for (let i = 0; i < data.length; i++) {
    const point = data[i];
    const ssr = point.ssr_output_percent;

    // Calculate temperature rate over window
    let rate = 0;
    if (i >= WINDOW_SIZE) {
      const timeDiff = (data[i].elapsed_seconds - data[i - WINDOW_SIZE].elapsed_seconds) / 60; // minutes
      const tempDiff = data[i].current_temp_c - data[i - WINDOW_SIZE].current_temp_c;
      rate = timeDiff > 0 ? tempDiff / timeDiff : 0;
    }

    // Determine phase type
    let phaseType: PhaseType;
    if (ssr < SSR_THRESHOLD) {
      phaseType = 'cooling';
    } else if (rate > RATE_THRESHOLD) {
      phaseType = 'heating';
    } else {
      phaseType = 'plateau';
    }

    // Check if we need to start a new phase
    if (!currentPhase || currentPhase.type !== phaseType) {
      // Close previous phase
      if (currentPhase) {
        currentPhase.endIdx = i - 1;
        currentPhase.tempEnd = data[i - 1].current_temp_c;
        phases.push(currentPhase);
      }

      // Start new phase
      currentPhase = {
        startIdx: i,
        endIdx: i,
        type: phaseType,
        avgSsr: ssr,
        tempStart: point.current_temp_c,
        tempEnd: point.current_temp_c,
        stepName: point.step_name,
      };
    } else {
      // Update running average for SSR
      const count = i - currentPhase.startIdx + 1;
      currentPhase.avgSsr = (currentPhase.avgSsr * (count - 1) + ssr) / count;
    }
  }

  // Close final phase
  if (currentPhase) {
    currentPhase.endIdx = data.length - 1;
    currentPhase.tempEnd = data[data.length - 1].current_temp_c;
    phases.push(currentPhase);
  }

  return phases;
}

const PHASE_COLORS: Record<PhaseType, string> = {
  heating: '#fca5a5',
  cooling: '#93c5fd',
  plateau: '#fef08a',
};

const PHASE_LABELS: Record<PhaseType, string> = {
  heating: 'Heating',
  cooling: 'Cooling',
  plateau: 'Plateau',
};

export function TuningPhasesVisualizer() {
  const [logData, setLogData] = useState<LogDataPoint[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleFileSelected = (content: string, filename: string) => {
    try {
      const { data } = parseLogCSV(content);
      
      if (data.length === 0) {
        throw new Error('No valid data points in log file');
      }

      // Verify this is tuning data
      const isTuning = data.some(d => d.state === 'TUNING');
      if (!isTuning) {
        console.warn('This log file may not be from a tuning run');
      }

      setLogData(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to parse log file');
      setLogData(null);
    }
  };

  const phases = useMemo<Phase[]>(() => {
    if (!logData) return [];
    return detectPhases(logData);
  }, [logData]);

  const chartData = useMemo<ChartDataPoint[]>(() => {
    if (!logData) return [];

    return logData.map((point, idx) => {
      const phase = phases.find(p => idx >= p.startIdx && idx <= p.endIdx);
      return {
        time_minutes: secondsToMinutes(point.elapsed_seconds),
        temp: point.current_temp_c,
        ssr_output: point.ssr_output_percent,
        phase: phase ? phase.type : undefined,
      };
    });
  }, [logData, phases]);

  const stats = useMemo(() => {
    if (chartData.length === 0) return null;

    const temps = chartData.map(p => p.temp);
    const maxTemp = Math.max(...temps);
    const minTemp = Math.min(...temps);
    const duration = chartData[chartData.length - 1].time_minutes;
    const startTime = logData?.[0]?.timestamp || '';

    return { maxTemp, minTemp, duration, startTime };
  }, [chartData, logData]);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Tuning Phases Visualizer</CardTitle>
        <CardDescription>
          Visualize PID tuning runs with physics-based phase detection
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <FileSourceSelector
          directory="logs"
          accept=".csv"
          onFileSelected={handleFileSelected}
          label="Select Tuning Log File"
          description="Choose a tuning log file to analyze"
        />

        {error && (
          <Alert variant="destructive">
            <AlertCircle className="h-4 w-4" />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {logData && chartData.length > 0 && (
          <div className="space-y-6 pt-6 border-t">
            <div>
              <h3 className="text-lg font-semibold">Tuning Phases - Physics-Based Detection</h3>
              {stats && (
                <p className="text-sm text-muted-foreground mt-1">
                  Started: {stats.startTime} | Duration: {stats.duration.toFixed(1)}min | 
                  Temp Range: {stats.minTemp.toFixed(1)}°C - {stats.maxTemp.toFixed(1)}°C
                </p>
              )}
            </div>

            <div className="space-y-6">
              <div className="flex gap-4 text-sm">
                <div className="flex items-center gap-2">
                  <div className="w-4 h-4 rounded" style={{ backgroundColor: PHASE_COLORS.heating, opacity: 0.3 }} />
                  <span>Heating (SSR on, temp rising)</span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-4 h-4 rounded" style={{ backgroundColor: PHASE_COLORS.cooling, opacity: 0.3 }} />
                  <span>Cooling (SSR off)</span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-4 h-4 rounded" style={{ backgroundColor: PHASE_COLORS.plateau, opacity: 0.3 }} />
                  <span>Plateau (SSR on, temp stable)</span>
                </div>
              </div>

              <ResponsiveContainer width="100%" height={400}>
                <LineChart data={chartData} margin={{ top: 5, right: 30, left: 20, bottom: 5 }}>
                  <CartesianGrid strokeDasharray="3 3" opacity={0.3} />
                  
                  {/* Draw phase backgrounds */}
                  {phases.map((phase, idx) => {
                    const startTime = secondsToMinutes(logData[phase.startIdx].elapsed_seconds);
                    const endTime = secondsToMinutes(logData[phase.endIdx].elapsed_seconds);
                    return (
                      <ReferenceArea
                        key={idx}
                        x1={startTime}
                        x2={endTime}
                        fill={PHASE_COLORS[phase.type]}
                        fillOpacity={0.3}
                      />
                    );
                  })}

                  <XAxis
                    dataKey="time_minutes"
                    label={{ value: 'Time (minutes)', position: 'insideBottom', offset: -5 }}
                  />
                  <YAxis
                    label={{ value: 'Temperature (°C)', angle: -90, position: 'insideLeft' }}
                  />
                  <Tooltip
                    formatter={(value: number) => [`${value.toFixed(1)}°C`, 'Temperature']}
                    labelFormatter={(label: number) => `Time: ${label.toFixed(1)}min`}
                  />
                  <Legend />
                  <Line
                    type="monotone"
                    dataKey="temp"
                    stroke="#3b82f6"
                    strokeWidth={2}
                    dot={false}
                    name="Temperature"
                  />
                </LineChart>
              </ResponsiveContainer>

              <div className="pt-6 border-t">
                <h4 className="text-base font-semibold mb-1">SSR Output with Phase Detection</h4>
                <p className="text-sm text-muted-foreground mb-4">
                  Solid State Relay duty cycle colored by detected phase
                </p>
                <ResponsiveContainer width="100%" height={250}>
                  <AreaChart data={chartData} margin={{ top: 5, right: 30, left: 20, bottom: 5 }}>
                    <CartesianGrid strokeDasharray="3 3" opacity={0.3} />
                    
                    {/* Draw phase backgrounds */}
                    {phases.map((phase, idx) => {
                      const startTime = secondsToMinutes(logData[phase.startIdx].elapsed_seconds);
                      const endTime = secondsToMinutes(logData[phase.endIdx].elapsed_seconds);
                      return (
                        <ReferenceArea
                          key={idx}
                          x1={startTime}
                          x2={endTime}
                          fill={PHASE_COLORS[phase.type]}
                          fillOpacity={0.3}
                        />
                      );
                    })}

                    <XAxis
                      dataKey="time_minutes"
                      label={{ value: 'Time (minutes)', position: 'insideBottom', offset: -5 }}
                    />
                    <YAxis
                      domain={[0, 100]}
                      label={{ value: 'SSR Output (%)', angle: -90, position: 'insideLeft' }}
                    />
                    <Tooltip
                      formatter={(value: number) => [`${value.toFixed(1)}%`, 'SSR Output']}
                      labelFormatter={(label: number) => `Time: ${label.toFixed(1)}min`}
                    />
                    <Legend />
                    <Area
                      type="monotone"
                      dataKey="ssr_output"
                      stroke="#f97316"
                      fill="#f97316"
                      fillOpacity={0.5}
                      name="SSR Output (%)"
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>

              <div className="pt-6 border-t">
                <h4 className="text-base font-semibold mb-1">Detected Phases Summary</h4>
                <p className="text-sm text-muted-foreground mb-4">
                  {phases.length} phases detected using physics-based algorithm
                </p>
                <div className="space-y-2">
                  {phases.map((phase, idx) => {
                    const startTime = secondsToMinutes(logData[phase.startIdx].elapsed_seconds);
                    const endTime = secondsToMinutes(logData[phase.endIdx].elapsed_seconds);
                    const duration = endTime - startTime;
                    const tempChange = phase.tempEnd - phase.tempStart;
                    const rate = duration > 0 ? (tempChange / duration) * 60 : 0; // °C/h

                    return (
                      <div
                        key={idx}
                        className="p-3 rounded border"
                        style={{ 
                          backgroundColor: PHASE_COLORS[phase.type], 
                          borderColor: PHASE_COLORS[phase.type],
                          opacity: 0.9 
                        }}
                      >
                        <div className="flex items-center justify-between">
                          <div className="font-semibold">
                            Phase {idx + 1}: {PHASE_LABELS[phase.type].toUpperCase()}
                            {phase.stepName && <span className="ml-2 text-sm font-normal">({phase.stepName})</span>}
                          </div>
                          <div className="text-sm">
                            {startTime.toFixed(1)} - {endTime.toFixed(1)} min ({duration.toFixed(1)} min)
                          </div>
                        </div>
                        <div className="mt-1 flex gap-4 text-sm">
                          <span>SSR: {phase.avgSsr.toFixed(1)}%</span>
                          <span>Temp: {phase.tempStart.toFixed(1)}°C → {phase.tempEnd.toFixed(1)}°C ({tempChange > 0 ? '+' : ''}{tempChange.toFixed(1)}°C)</span>
                          <span>Rate: {rate > 0 ? '+' : ''}{rate.toFixed(1)}°C/h</span>
                        </div>
                      </div>
                    );
                  })}
                </div>

                <div className="mt-4 p-3 bg-muted/50 rounded text-sm">
                  <div className="font-semibold mb-1">Phase Classification Logic:</div>
                  <ul className="space-y-1 text-muted-foreground">
                    <li>• <strong>COOLING:</strong> SSR &lt; 5% (natural cooling, no heat input)</li>
                    <li>• <strong>HEATING:</strong> SSR ≥ 5% AND temp rising &gt; 0.5°C/min</li>
                    <li>• <strong>PLATEAU:</strong> SSR ≥ 5% AND temp stable ±0.5°C/min</li>
                  </ul>
                </div>
              </div>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
