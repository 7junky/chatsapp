up:
	docker-compose up -d
down:
	docker-compose down
build:
	docker-compose build --no-cache
clean:
	docker image prune

redis:
	docker exec -it chatsapp-redis-1 redis-cli -a redis
