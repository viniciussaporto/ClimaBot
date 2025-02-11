FROM node:18-bullseye-slim

WORKDIR /usr/src/app

# Install system dependencies
RUN apt-get update && \
    apt-get install -y \
    python3 \
    build-essential \
    libcairo2-dev \
    libpango1.0-dev \
    libjpeg-dev \
    libgif-dev \
    librsvg2-dev

COPY package*.json ./
COPY tsconfig*.json ./

RUN npm ci --production

COPY . .

RUN npm run build

EXPOSE 9464

CMD ["npm", "start"]