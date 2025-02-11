FROM node:18-alpine

# set working directory
WORKDIR /usr/src/app

# package files first for better layer caching
COPY package*.json ./
COPY tsconfig*.json ./

# Install dependencies
RUN npm ci

COPY . .

RUN npm run build

# expose metrics port
EXPOSE 9464

# start bot
CMD ["node", "dist/index.js"]