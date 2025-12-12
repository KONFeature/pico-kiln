// Profile cache context for preloading and caching profile data
// Ensures profiles are loaded sequentially to avoid overloading the Pico

import React, {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useRef,
	useState,
	type ReactNode,
} from "react";
import { useQueryClient } from "@tanstack/react-query";
import { usePico } from "./context";
import { picoKeys, useListFiles } from "./hooks";
import type { Profile } from "./types";

interface ProfileCacheContextValue {
	// Get a cached profile by name (without .json extension)
	getProfile: (name: string) => Profile | undefined;
	// Check if a profile is loaded
	isProfileLoaded: (name: string) => boolean;
	// Loading state
	isPreloading: boolean;
	preloadProgress: { loaded: number; total: number };
}

const ProfileCacheContext = createContext<ProfileCacheContextValue | undefined>(
	undefined,
);

interface ProfileCacheProviderProps {
	children: ReactNode;
}

export function ProfileCacheProvider({ children }: ProfileCacheProviderProps) {
	const { client, isConfigured } = usePico();
	const queryClient = useQueryClient();
	const { data: profilesData } = useListFiles("profiles");

	// Track loaded profiles in state
	const [loadedProfiles, setLoadedProfiles] = useState<Map<string, Profile>>(
		new Map(),
	);
	const [isPreloading, setIsPreloading] = useState(false);
	const [preloadProgress, setPreloadProgress] = useState({ loaded: 0, total: 0 });

	// Track which profiles we've already attempted to load
	const loadedNamesRef = useRef<Set<string>>(new Set());
	const isLoadingRef = useRef(false);

	// Extract profile names from the file list
	const profileNames =
		profilesData?.files
			.filter((file) => file.name.endsWith(".json"))
			.map((file) => file.name.replace(".json", "")) || [];

	// Sequential profile preloader
	useEffect(() => {
		if (!client || !isConfigured || profileNames.length === 0) {
			return;
		}

		// Find profiles we haven't loaded yet
		const unloadedProfiles = profileNames.filter(
			(name) => !loadedNamesRef.current.has(name),
		);

		if (unloadedProfiles.length === 0 || isLoadingRef.current) {
			return;
		}

		// Start sequential loading
		const loadProfilesSequentially = async () => {
			isLoadingRef.current = true;
			setIsPreloading(true);
			setPreloadProgress({ loaded: 0, total: unloadedProfiles.length });

			for (let i = 0; i < unloadedProfiles.length; i++) {
				const profileName = unloadedProfiles[i];
				const filename = `${profileName}.json`;

				try {
					// Check if already in TanStack Query cache
					const cachedData = queryClient.getQueryData<{ content: string }>(
						picoKeys.fileContent("profiles", filename),
					);

					let profileContent: string;

					if (cachedData?.content) {
						profileContent = cachedData.content;
					} else {
						// Fetch from API
						const response = await client.getFile("profiles", filename);
						if (response.success && response.content) {
							profileContent = response.content;
							// Update TanStack Query cache
							queryClient.setQueryData(
								picoKeys.fileContent("profiles", filename),
								response,
							);
						} else {
							loadedNamesRef.current.add(profileName);
							setPreloadProgress({ loaded: i + 1, total: unloadedProfiles.length });
							continue;
						}
					}

					// Parse the profile
					const profile: Profile = JSON.parse(profileContent);

					// Add to loaded profiles
					setLoadedProfiles((prev) => {
						const newMap = new Map(prev);
						newMap.set(profileName, profile);
						return newMap;
					});

					loadedNamesRef.current.add(profileName);
				} catch (error) {
					console.error(`Failed to load profile ${profileName}:`, error);
					loadedNamesRef.current.add(profileName);
				}

				setPreloadProgress({ loaded: i + 1, total: unloadedProfiles.length });

				// Small delay between requests to be gentle on the Pico
				if (i < unloadedProfiles.length - 1) {
					await new Promise((resolve) => setTimeout(resolve, 100));
				}
			}

			isLoadingRef.current = false;
			setIsPreloading(false);
		};

		loadProfilesSequentially();
	}, [client, isConfigured, profileNames, queryClient]);

	const getProfile = useCallback(
		(name: string): Profile | undefined => {
			return loadedProfiles.get(name);
		},
		[loadedProfiles],
	);

	const isProfileLoaded = useCallback(
		(name: string): boolean => {
			return loadedProfiles.has(name);
		},
		[loadedProfiles],
	);

	const value: ProfileCacheContextValue = {
		getProfile,
		isProfileLoaded,
		isPreloading,
		preloadProgress,
	};

	return (
		<ProfileCacheContext.Provider value={value}>
			{children}
		</ProfileCacheContext.Provider>
	);
}

export function useProfileCache(): ProfileCacheContextValue {
	const context = useContext(ProfileCacheContext);
	if (!context) {
		throw new Error(
			"useProfileCache must be used within a ProfileCacheProvider",
		);
	}
	return context;
}
