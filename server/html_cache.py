# server/html_cache.py
# In-memory HTML file cache to eliminate blocking file I/O during requests
#
# This module pre-loads static HTML files at startup and serves them from RAM,
# preventing event loop blocking during request handling.

import gc

class HTMLCache:
    """
    Singleton cache for static HTML files

    Pre-loads HTML files into memory at startup to avoid blocking file I/O
    during request handling, which would freeze the async event loop.
    """

    _instance = None

    def __new__(cls):
        """Singleton pattern - only one instance allowed"""
        if cls._instance is None:
            cls._instance = super(HTMLCache, cls).__new__(cls)
            cls._instance._initialized = False
        return cls._instance

    def __init__(self):
        """Initialize the HTML cache (only runs once due to singleton)"""
        if self._initialized:
            return

        self._cache = {}
        self._initialized = True
        print("[HTMLCache] Singleton instance created")

    def preload(self, files):
        """
        Pre-load HTML files into memory

        Args:
            files: Dictionary mapping cache keys to file paths
                   e.g., {'index': 'static/index.html', 'tuning': 'static/tuning.html'}

        Returns:
            Number of files successfully loaded
        """
        loaded = 0

        for key, filepath in files.items():
            try:
                with open(filepath, 'r') as f:
                    content = f.read()

                self._cache[key] = content
                size_kb = len(content) / 1024
                print(f"[HTMLCache] Loaded '{key}' from {filepath} ({size_kb:.1f} KB)")
                loaded += 1

            except OSError as e:
                print(f"[HTMLCache] WARNING: Failed to load {filepath}: {e}")

        # Force GC after loading to clean up temporary objects
        gc.collect()
        free_mem = gc.mem_free()
        print(f"[HTMLCache] Pre-loaded {loaded}/{len(files)} files, free memory: {free_mem}")

        return loaded

    def render_profiles_list(self, profile_names):
        """
        Render profiles list HTML for index page

        Args:
            profile_names: List of profile names

        Returns:
            HTML string for profiles list
        """
        if not profile_names:
            return '<ul><li>No profiles found</li></ul>'

        parts = ['<ul>']
        for name in profile_names:
            parts.append(f'<li>{name} <button onclick="startProfile(\'{name}\')">Start</button></li>')
        parts.append('</ul>')

        return ''.join(parts)

    def render_template(self, key, replacements):
        """
        Render cached template with replacements

        Args:
            key: Cache key (e.g., 'index')
            replacements: Dict of {placeholder: value} to replace

        Returns:
            Rendered HTML string, or None if key not found
        """
        template = self._cache.get(key)
        if not template:
            return None

        html = template
        for placeholder, value in replacements.items():
            html = html.replace(placeholder, value)

        return html

    def prerender(self, key, replacements):
        """
        Pre-render a template and cache the result

        Args:
            key: Cache key for template
            replacements: Dict of {placeholder: value} to replace

        Returns:
            True if successful, False otherwise
        """
        rendered = self.render_template(key, replacements)
        if rendered:
            self._cache[key] = rendered
            print(f"[HTMLCache] Pre-rendered '{key}' with {len(replacements)} replacements")
            return True
        return False

    def get(self, key):
        """
        Get cached HTML content

        Args:
            key: Cache key (e.g., 'index', 'tuning')

        Returns:
            HTML content string, or None if not found
        """
        return self._cache.get(key)

    def clear(self):
        """Clear all cached content (frees memory)"""
        count = len(self._cache)
        self._cache.clear()
        gc.collect()
        print(f"[HTMLCache] Cleared {count} cached files")


# Global singleton instance
_cache = HTMLCache()


def get_html_cache():
    """
    Get the global HTMLCache singleton instance

    Returns:
        HTMLCache instance
    """
    return _cache
