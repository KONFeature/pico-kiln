// Context for managing Pico connection state and API client

import React, { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from 'react';
import { PicoAPIClient } from './client';
import type { ConnectionHealth } from './types';

const STORAGE_KEY = 'pico-kiln-url';
const DEFAULT_URL = '';

interface PicoContextValue {
  // Connection state
  picoURL: string;
  setPicoURL: (url: string) => void;
  isConfigured: boolean;
  
  // API client
  client: PicoAPIClient | null;
  
  // Connection health
  connectionHealth: ConnectionHealth;
  updateConnectionHealth: (success: boolean, error?: string) => void;
  testConnection: () => Promise<boolean>;
  
  // Reset
  reset: () => void;
}

const PicoContext = createContext<PicoContextValue | undefined>(undefined);

interface PicoProviderProps {
  children: ReactNode;
}

export function PicoProvider({ children }: PicoProviderProps) {
  const [picoURL, setPicoURLState] = useState<string>(() => {
    // Load from localStorage on mount (only on client side)
    if (typeof window !== 'undefined') {
      return localStorage.getItem(STORAGE_KEY) || DEFAULT_URL;
    }
    return DEFAULT_URL;
  });

  const [client, setClient] = useState<PicoAPIClient | null>(() => {
    const savedURL = typeof window !== 'undefined' 
      ? localStorage.getItem(STORAGE_KEY) || DEFAULT_URL
      : DEFAULT_URL;
    return savedURL ? new PicoAPIClient(savedURL) : null;
  });

  const [connectionHealth, setConnectionHealth] = useState<ConnectionHealth>({
    connected: false,
    consecutiveFailures: 0,
  });

  // Update localStorage when URL changes
  const setPicoURL = useCallback((url: string) => {
    const trimmedURL = url.trim();
    setPicoURLState(trimmedURL);
    
    if (typeof window !== 'undefined') {
      if (trimmedURL) {
        localStorage.setItem(STORAGE_KEY, trimmedURL);
      } else {
        localStorage.removeItem(STORAGE_KEY);
      }
    }

    // Update or create client
    if (trimmedURL) {
      setClient(new PicoAPIClient(trimmedURL));
      // Reset connection health when URL changes
      setConnectionHealth({
        connected: false,
        consecutiveFailures: 0,
      });
    } else {
      setClient(null);
      setConnectionHealth({
        connected: false,
        consecutiveFailures: 0,
      });
    }
  }, []);

  // Update connection health based on API call results
  const updateConnectionHealth = useCallback((success: boolean, error?: string) => {
    setConnectionHealth((prev) => {
      if (success) {
        return {
          connected: true,
          lastSuccessfulRequest: Date.now(),
          consecutiveFailures: 0,
        };
      }
      
      return {
        connected: false,
        lastSuccessfulRequest: prev.lastSuccessfulRequest,
        consecutiveFailures: prev.consecutiveFailures + 1,
        lastError: error,
      };
    });
  }, []);

  // Test connection manually
  const testConnection = useCallback(async (): Promise<boolean> => {
    if (!client) return false;

    try {
      const isConnected = await client.testConnection();
      updateConnectionHealth(isConnected, isConnected ? undefined : 'Connection test failed');
      return isConnected;
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      updateConnectionHealth(false, errorMessage);
      return false;
    }
  }, [client, updateConnectionHealth]);

  // Reset all state
  const reset = useCallback(() => {
    setPicoURL('');
  }, [setPicoURL]);

  const value: PicoContextValue = {
    picoURL,
    setPicoURL,
    isConfigured: Boolean(picoURL),
    client,
    connectionHealth,
    updateConnectionHealth,
    testConnection,
    reset,
  };

  return <PicoContext.Provider value={value}>{children}</PicoContext.Provider>;
}

export function usePico(): PicoContextValue {
  const context = useContext(PicoContext);
  if (!context) {
    throw new Error('usePico must be used within a PicoProvider');
  }
  return context;
}

// Hook to ensure Pico is configured (for protected pages)
export function useRequirePico(): PicoContextValue {
  const pico = usePico();
  
  useEffect(() => {
    if (!pico.isConfigured) {
      console.warn('Pico URL is not configured');
    }
  }, [pico.isConfigured]);

  return pico;
}
