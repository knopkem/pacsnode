/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        surface: {
          950: '#020617',
          925: '#050b18',
          900: '#0f172a',
          850: '#111b2e',
          800: '#172033',
        },
      },
      boxShadow: {
        panel: '0 24px 80px rgba(2, 6, 23, 0.45)',
      },
    },
  },
  plugins: [],
}
