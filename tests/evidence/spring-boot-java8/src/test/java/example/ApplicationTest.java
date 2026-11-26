package example;

import java.nio.file.Files;
import java.nio.file.Paths;
import org.junit.Test;
import org.springframework.boot.SpringApplication;
import org.springframework.context.ConfigurableApplicationContext;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
import static org.junit.Assert.assertEquals;

public class ApplicationTest {
    @Configuration
    public static class Fixture {
        @Bean public String greeting() { return "hello"; }
    }

    @Test public void bootsWithSelectedJava8() {
        assertEquals("1.8", System.getProperty("java.specification.version"));
        try (ConfigurableApplicationContext context = SpringApplication.run(Fixture.class)) {
            String expected = Files.exists(Paths.get("fail-assertion")) ? "intentional mismatch" : "hello";
            assertEquals(expected, context.getBean("greeting"));
        }
    }
}
