# Mizan Thin UI (React + RTK)

Lightweight React shell for admin/user dashboard operations.

## Stack

- React + TypeScript
- Vite
- Tailwind CSS
- Redux Toolkit + RTK Query
- React Router

## Run

```bash
cd ui
npm install
cp .env.example .env
npm run dev
```

App default expects backend API at `http://127.0.0.1:18180`.

## What is included today

- Login/Register
- Protected admin and user routes
- Admin pages: provider connections, model routes, usage
- User pages: key management, usage, wallet summary
- Centralized RTK Query API layer
