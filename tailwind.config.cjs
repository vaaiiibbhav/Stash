/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./index.html",
    // .fs sources carry the Tailwind classes (Fable/F# frontend)
    "./src/**/*.{fs,js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {},
  },
  plugins: [],
}
